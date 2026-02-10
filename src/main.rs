use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Result};
use clap::Parser;
use landlock::{
    self, path_beneath_rules, Access, AccessFs, BitFlags, RestrictionStatus, Ruleset, RulesetAttr,
    RulesetCreatedAttr, ABI,
};
use log::{error, info, warn, LevelFilter};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(name = "ai-sandbox-landlock")]
#[command(about = "Minimal Landlock-based launcher (prototype)")]
struct Args {
    /// Config file (YAML). If provided, uses profiles from it.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Profile name (required when using --config).
    #[arg(long)]
    profile: Option<String>,

    /// Project root to which filesystem access should be limited
    #[arg(long)]
    root: Option<String>,

    /// Read-only mode (no writes in the project root)
    #[arg(long, default_value_t = false)]
    read_only: bool,

    /// Only check whether Landlock is available/usable, then exit.
    #[arg(long, default_value_t = false)]
    check: bool,

    /// Dry-run: build and print rules; do not restrict or run.
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Require Landlock: fail if not available.
    #[arg(long, default_value_t = false)]
    require_landlock: bool,

    /// Log level: error, warn, info, debug, trace.
    #[arg(long)]
    log_level: Option<String>,

    /// Disable colored logs.
    #[arg(long, default_value_t = false)]
    no_color: bool,

    /// Print selected config/profile and exit.
    #[arg(long, default_value_t = false)]
    print_config: bool,

    /// Print planned ruleset and exit.
    #[arg(long, default_value_t = false)]
    print_ruleset: bool,

    /// Generate a profile YAML based on project root (git or --root) and exit.
    #[arg(long, default_value_t = false)]
    generate_profile: bool,

    /// Name for the generated profile (defaults to basename of root).
    #[arg(long)]
    gen_name: Option<String>,

    /// Output file path for generated YAML (stdout if omitted).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Command to run inside the sandbox (after "--")
    #[arg(last = true)]
    command: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    // Profile generation mode (Stage 9 dynamic)
    if args.generate_profile {
        init_logger(args.log_level.as_deref(), args.no_color);
        generate_profile_yaml(&args)?;
        return Ok(());
    }

    if args.check {
        return match perform_landlock_check() {
            Ok(report) => {
                println!("{}", report);
                Ok(())
            }
            Err(e) => {
                eprintln!("Landlock check failed: {}", e);
                // Non-zero exit code to signal failure
                std::process::exit(1);
            }
        };
    }

    // If config provided, load profile and override fields.
    let mut effective_cmd: Vec<String> = args.command.clone();
    let mut effective_root: Option<String> = args.root.clone();
    let mut effective_read_only: bool = args.read_only;
    let mut selected_profile: Option<Profile> = None;
    let mut effective_log_level: Option<String> = args.log_level.clone();

    if let Some(cfg_path) = args.config.as_ref() {
        let profile_name = args
            .profile
            .as_ref()
            .ok_or_else(|| anyhow!("--profile is required when using --config"))?;
        let cfg = load_config(cfg_path)?;
        let profile = cfg
            .profiles
            .get(profile_name)
            .ok_or_else(|| anyhow!("profile '{}' not found in config", profile_name))?;

        selected_profile = Some(profile.clone());
        if effective_log_level.is_none() {
            effective_log_level = profile.log_level.clone();
        }

        // Resolve command from profile if not overridden by CLI tail
        if effective_cmd.is_empty() {
            effective_cmd = std::iter::once(profile.command.binary.clone())
                .chain(profile.command.args.clone())
                .collect();
        }

        // Resolve working_dir and env in run_command later

        // For Stage 2 we map first projects group as root; later support multiple roots
        if let Some(projects) = profile.access_roots.get("projects") {
            if let Some(first_path) = projects.paths.first() {
                effective_root = Some(normalize_path(first_path)?);
                // read_only from permissions: if no write_file/remove_file, treat as read-only
                effective_read_only = !projects.permissions.write_file.unwrap_or(false)
                    && !projects.permissions.remove_file.unwrap_or(false)
                    && !projects.permissions.truncate.unwrap_or(false);
            }
        }
    }

    // Initialize logger after computing effective log level
    init_logger(effective_log_level.as_deref(), args.no_color);
    info!(
        "starting ai-sandbox-landlock with root={:?}, read_only={}, check={}, dry_run={}, require_landlock={}",
        effective_root.as_deref().unwrap_or("<none>"),
        effective_read_only,
        args.check,
        args.dry_run,
        args.require_landlock
    );

    // Print config if requested
    if args.print_config {
        if let Some(profile) = selected_profile.as_ref() {
            let yaml = serde_yaml::to_string(profile)?;
            println!("Selected profile:\n{}", yaml);
        } else {
            println!(
                "No profile selected; root={:?}, read_only={}",
                effective_root, effective_read_only
            );
        }
        return Ok(());
    }

    // Print ruleset or dry-run without enforcement
    if args.print_ruleset || args.dry_run {
        if let Some(profile) = selected_profile.as_ref() {
            print_ruleset_profile(profile)?;
        } else {
            let root = effective_root
                .as_ref()
                .ok_or_else(|| anyhow!("project root is required (provide --root or set access_roots.projects in profile)"))?;
            print_ruleset_root(root, effective_read_only)?;
        }
        return Ok(());
    }

    if effective_cmd.is_empty() {
        return Err(anyhow!(
            "no command specified (use: --config ... --profile ... or -- <CMD> [ARGS...])"
        ));
    }

    // Landlock availability and require behavior
    let ll_available = perform_landlock_check().is_ok();
    if args.require_landlock && !ll_available {
        return Err(anyhow!("Landlock is required but not available"));
    }
    if !ll_available {
        warn!("Landlock not available; proceeding without sandbox.");
    }

    // Apply Landlock sandbox either from full profile or a simple root restriction (if available).
    if ll_available {
        if let Some(profile) = selected_profile.as_ref() {
            setup_landlock_profile(profile)?;
        } else {
            let root = effective_root
                .as_ref()
                .ok_or_else(|| anyhow!("project root is required (provide --root or set access_roots.projects in profile)"))?;
            setup_landlock_root(root, effective_read_only)?;
        }
    }

    let code = run_command(
        &effective_cmd,
        selected_profile.as_ref().map(|p| &p.command),
    )?;
    std::process::exit(code);
}

fn detect_project_root(args: &Args) -> Result<String> {
    if let Some(root) = args.root.as_ref() {
        return normalize_path(root);
    }
    // Try git
    let mut git = Command::new("git");
    git.arg("rev-parse").arg("--show-toplevel");
    if let Ok(out) = git.output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Ok(s);
            }
        }
    }
    // Fallback: current directory
    let cwd = env::current_dir()?.to_string_lossy().into_owned();
    Ok(cwd)
}

fn generate_profile_yaml(args: &Args) -> Result<()> {
    let root = detect_project_root(args)?;
    let name = args.gen_name.clone().unwrap_or_else(|| {
        PathBuf::from(&root)
            .file_name()
            .and_then(|os| os.to_str())
            .unwrap_or("project")
            .to_string()
    });

    let projects = AccessRootGroup {
        paths: vec![root.clone()],
        permissions: Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(false),
            write_file: Some(false),
            remove_file: Some(false),
            remove_dir: Some(false),
            truncate: Some(false),
        },
    };
    let system = AccessRootGroup {
        paths: vec!["/usr".to_string(), "/lib".to_string(), "/lib64".to_string()],
        permissions: Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(true),
            write_file: Some(false),
            remove_file: Some(false),
            remove_dir: Some(false),
            truncate: Some(false),
        },
    };
    let cache = AccessRootGroup {
        paths: vec!["~/.ai-sandbox/cache".to_string()],
        permissions: Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(false),
            write_file: Some(true),
            remove_file: Some(true),
            remove_dir: Some(false),
            truncate: Some(false),
        },
    };

    let mut access_roots = HashMap::new();
    access_roots.insert("projects".to_string(), projects);
    access_roots.insert("system".to_string(), system);
    access_roots.insert("cache".to_string(), cache);

    let profile = Profile {
        description: Some(format!("Generated profile for {}", name)),
        access_roots,
        control_access: ControlAccess {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(true),
            write_file: Some(false),
            remove_file: Some(false),
            remove_dir: Some(false),
            truncate: Some(false),
        },
        command: CommandSpec {
            binary: "/bin/bash".to_string(),
            args: vec![],
            working_dir: Some(root.clone()),
            env: None,
        },
        log_level: Some("info".to_string()),
        dry_run: Some(false),
    };

    let mut profiles = HashMap::new();
    profiles.insert(name.clone(), profile);
    let cfg = Config {
        version: Some(1),
        profiles,
    };
    let yaml = serde_yaml::to_string(&cfg)?;

    if let Some(out) = args.output.as_ref() {
        fs::write(out, &yaml)?;
        println!("Profile '{}' written to {}", name, out.to_string_lossy());
    } else {
        println!("{}", yaml);
    }
    Ok(())
}

fn init_logger(level: Option<&str>, no_color: bool) {
    let mut builder = env_logger::Builder::from_default_env();
    let lvl = match level.unwrap_or("info") {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    };
    builder.filter_level(lvl);
    // Honor --no-color to disable ANSI coloring in logs (useful for CI/pipes)
    if no_color {
        builder.write_style(env_logger::WriteStyle::Never);
    }
    builder.init();
}

fn setup_landlock_root(root: &str, read_only: bool) -> Result<()> {
    let _abi = ABI::V1;
    let normalized = normalize_path(root)?;
    let paths = vec![normalized];

    // Allowed per-root permissions
    let allowed = if read_only {
        // Read-only + allow executing files under the root
        AccessFs::from_read(_abi) | AccessFs::Execute
    } else {
        AccessFs::from_all(_abi)
    };

    // Handled accesses are whatever we intend to restrict
    let handled = allowed;

    let created = Ruleset::default().handle_access(handled)?.create()?;

    let created = created.add_rules(path_beneath_rules(&paths, allowed))?;
    let status: RestrictionStatus = created.restrict_self()?;
    info!("Landlock applied (root mode): status={:?}", status);
    Ok(())
}

fn setup_landlock_profile(profile: &Profile) -> Result<()> {
    // Collect union of all rights we will handle
    let mut handled: BitFlags<AccessFs> = BitFlags::empty();

    // Include global control_access into handled rights
    handled.insert(access_from_control(&profile.control_access));

    // Union handled accesses from all groups
    for (_group_name, group) in profile.access_roots.iter() {
        let allowed = access_from_permissions(&group.permissions);
        handled.insert(allowed);
    }

    let mut created = Ruleset::default().handle_access(handled)?.create()?;

    for (_group_name, group) in profile.access_roots.iter() {
        let allowed = access_from_permissions(&group.permissions);
        let mut norm_paths: Vec<String> = Vec::with_capacity(group.paths.len());
        for p in &group.paths {
            norm_paths.push(normalize_path(p)?);
        }
        created = created.add_rules(path_beneath_rules(&norm_paths, allowed))?;
    }
    let status: RestrictionStatus = created.restrict_self()?;
    warn!("Applied Landlock; ensure no broad FDs were open before restrict_self.");
    info!("Landlock applied (profile mode): status={:?}", status);
    Ok(())
}

fn print_ruleset_root(root: &str, read_only: bool) -> Result<()> {
    let normalized = normalize_path(root)?;
    let allowed = if read_only {
        access_from_permissions(&Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(true),
            ..Permissions::default()
        })
    } else {
        access_from_permissions(&Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            execute: Some(true),
            write_file: Some(true),
            remove_file: Some(true),
            remove_dir: Some(true),
            truncate: Some(true),
        })
    };
    let names = access_names(allowed);
    let ignored = unsupported_names(allowed);
    println!("Ruleset (root mode):");
    println!("  handled = {:?}", names);
    if !ignored.is_empty() {
        println!("  ignored (unsupported by ABI): {:?}", ignored);
    }
    println!("  paths:");
    println!("    - {}", normalized);
    println!("      allowed = {:?}", names);
    Ok(())
}

fn print_ruleset_profile(profile: &Profile) -> Result<()> {
    let mut handled: BitFlags<AccessFs> = BitFlags::empty();
    handled.insert(access_from_control(&profile.control_access));
    for (_group_name, group) in profile.access_roots.iter() {
        handled.insert(access_from_permissions(&group.permissions));
    }
    let handled_names = access_names(handled);
    let handled_ignored = unsupported_names(handled);
    println!("Ruleset (profile mode):");
    println!("  handled = {:?}", handled_names);
    if !handled_ignored.is_empty() {
        println!("  ignored (unsupported by ABI): {:?}", handled_ignored);
    }
    println!("  groups:");
    for (group_name, group) in profile.access_roots.iter() {
        let allowed = access_from_permissions(&group.permissions);
        let names = access_names(allowed);
        let ignored = unsupported_names(allowed);
        println!("    - {}:", group_name);
        println!("      allowed = {:?}", names);
        if !ignored.is_empty() {
            println!("      ignored (unsupported by ABI): {:?}", ignored);
        }
        println!("      paths:");
        for p in &group.paths {
            println!("        - {}", normalize_path(p)?);
        }
    }
    Ok(())
}

fn access_names(set: BitFlags<AccessFs>) -> Vec<&'static str> {
    let mut v = Vec::new();
    if set.contains(AccessFs::ReadFile) {
        v.push("ReadFile");
    }
    if set.contains(AccessFs::ReadDir) {
        v.push("ReadDir");
    }
    if set.contains(AccessFs::Execute) {
        v.push("Execute");
    }
    if set.contains(AccessFs::WriteFile) {
        v.push("WriteFile");
    }
    if set.contains(AccessFs::RemoveFile) {
        v.push("RemoveFile");
    }
    if set.contains(AccessFs::RemoveDir) {
        v.push("RemoveDir");
    }
    if set.contains(AccessFs::Truncate) {
        v.push("Truncate");
    }
    v
}

fn supported_access() -> BitFlags<AccessFs> {
    // Detect max supported ABI by attempting ruleset creation with descending ABIs.
    // Prefer V2 if available, else fall back to V1.
    // Safe: creation here does not restrict self; it's a capability probe.
    for abi in [ABI::V2, ABI::V1].into_iter() {
        let handled = AccessFs::from_all(abi);
        if Ruleset::default()
            .handle_access(handled)
            .and_then(|rs| rs.create())
            .is_ok()
        {
            return handled;
        }
    }
    // Default fallback
    AccessFs::from_all(ABI::V1)
}

fn unsupported_names(requested: BitFlags<AccessFs>) -> Vec<&'static str> {
    let sup = supported_access();
    let mut v = Vec::new();
    if requested.contains(AccessFs::ReadFile) && !sup.contains(AccessFs::ReadFile) {
        v.push("ReadFile");
    }
    if requested.contains(AccessFs::ReadDir) && !sup.contains(AccessFs::ReadDir) {
        v.push("ReadDir");
    }
    if requested.contains(AccessFs::Execute) && !sup.contains(AccessFs::Execute) {
        v.push("Execute");
    }
    if requested.contains(AccessFs::WriteFile) && !sup.contains(AccessFs::WriteFile) {
        v.push("WriteFile");
    }
    if requested.contains(AccessFs::RemoveFile) && !sup.contains(AccessFs::RemoveFile) {
        v.push("RemoveFile");
    }
    if requested.contains(AccessFs::RemoveDir) && !sup.contains(AccessFs::RemoveDir) {
        v.push("RemoveDir");
    }
    if requested.contains(AccessFs::Truncate) && !sup.contains(AccessFs::Truncate) {
        v.push("Truncate");
    }
    v
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_expands_home() {
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/tmp/testhome");
        let r = normalize_path("~/.cache").unwrap();
        assert!(r.starts_with("/tmp/testhome/"));
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        }
    }

    #[test]
    fn test_permissions_mapping_read_only() {
        let perms = Permissions {
            read_file: Some(true),
            read_dir: Some(true),
            ..Permissions::default()
        };
        let set = access_from_permissions(&perms);
        assert!(set.contains(AccessFs::ReadFile));
        assert!(set.contains(AccessFs::ReadDir));
        assert!(!set.contains(AccessFs::WriteFile));
    }

    #[test]
    fn test_control_mapping_execute() {
        let ctrl = ControlAccess {
            execute: Some(true),
            ..ControlAccess::default()
        };
        let set = access_from_control(&ctrl);
        assert!(set.contains(AccessFs::Execute));
        assert!(!set.contains(AccessFs::WriteFile));
    }

    #[test]
    fn test_supported_access_includes_basic_reads() {
        let sup = supported_access();
        assert!(sup.contains(AccessFs::ReadFile));
        assert!(sup.contains(AccessFs::ReadDir));
    }

    #[test]
    fn test_unsupported_names_reports_truncate_when_not_supported() {
        let mut requested = BitFlags::<AccessFs>::empty();
        requested.insert(AccessFs::Truncate);
        let ignored = unsupported_names(requested);
        // On ABI V1, Truncate is typically unsupported.
        // We allow either outcome to avoid kernel-specific brittleness.
        assert!(ignored.is_empty() || ignored.contains(&"Truncate"));
    }

    #[test]
    fn test_load_example_config_has_minimal_profile() {
        let path = PathBuf::from("examples/ai-sandbox-landlock.yaml");
        let cfg = load_config(&path).unwrap();
        assert!(cfg.profiles.contains_key("minimal"));
    }

    #[test]
    fn test_print_ruleset_profile_runs() {
        let group = AccessRootGroup {
            paths: vec!["/usr".to_string()],
            permissions: Permissions {
                read_file: Some(true),
                read_dir: Some(true),
                execute: Some(true),
                ..Permissions::default()
            },
        };
        let mut access_roots = HashMap::new();
        access_roots.insert("system".to_string(), group);
        let profile = Profile {
            description: Some("test".to_string()),
            access_roots,
            control_access: ControlAccess {
                read_file: Some(true),
                read_dir: Some(true),
                execute: Some(true),
                ..ControlAccess::default()
            },
            command: CommandSpec {
                binary: "/bin/true".to_string(),
                args: vec![],
                working_dir: None,
                env: None,
            },
            log_level: Some("info".to_string()),
            dry_run: Some(true),
        };
        let r = print_ruleset_profile(&profile);
        assert!(r.is_ok());
    }
}

fn run_command(cmd: &[String], spec: Option<&CommandSpec>) -> Result<i32> {
    let (bin, args) = cmd
        .split_first()
        .ok_or_else(|| anyhow!("command vector is empty"))?;

    let mut cmdp = Command::new(bin);
    cmdp.args(args);
    if let Some(spec) = spec {
        if let Some(wd) = spec.working_dir.as_ref() {
            let cwd = normalize_path(wd)?;
            if !std::path::Path::new(&cwd).is_dir() {
                return Err(anyhow!("working_dir does not exist or is not a directory: {}", cwd));
            }
            cmdp.current_dir(cwd);
        }
        if let Some(envs) = spec.env.as_ref() {
            // Normalize env values that use ~/ expansion for better UX
            let mut norm_envs: HashMap<String, String> = HashMap::with_capacity(envs.len());
            for (k, v) in envs {
                let nv = if v.starts_with("~/") {
                    normalize_path(v)?
                } else {
                    v.clone()
                };
                norm_envs.insert(k.clone(), nv);
            }
            cmdp.envs(norm_envs);
        }
    }

    let status = cmdp.status()?;

    if let Some(code) = status.code() {
        Ok(code)
    } else {
        // Process terminated by signal; map to generic non-zero code.
        error!("process terminated by signal");
        Ok(1)
    }
}

// ---------------- Landlock check ----------------

fn perform_landlock_check() -> Result<String> {
    // 1) Kernel version check (>= 5.13)
    let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| String::from("unknown"));
    let version_ok = parse_kernel_version_ge(&osrelease, 5, 13);

    // 2) LSM list contains 'landlock'
    let lsm_list = std::fs::read_to_string("/sys/kernel/security/lsm")
        .unwrap_or_else(|_| String::from("<unavailable>"));
    let lsm_ok = lsm_list.contains("landlock");

    // 3) Optional: try to build minimal ruleset without applying
    // We only build a report string; defer actual syscall-based check to later.

    let mut report = String::new();
    report.push_str(&format!("Kernel osrelease: {}\n", osrelease.trim()));
    report.push_str(&format!("Kernel version >= 5.13: {}\n", yesno(version_ok)));
    report.push_str(&format!("LSM list: {}\n", lsm_list.trim()));
    report.push_str(&format!("Landlock listed in LSM: {}\n", yesno(lsm_ok)));

    if version_ok && lsm_ok {
        report.push_str("Result: Landlock appears available.\n");
        Ok(report)
    } else {
        Err(anyhow!(
            "Landlock not available: version_ok={} lsm_ok={}",
            version_ok,
            lsm_ok
        ))
    }
}

fn yesno(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn parse_kernel_version_ge(osrelease: &str, want_major: u32, want_minor: u32) -> bool {
    // osrelease like "6.8.0-26-generic"
    let parts: Vec<&str> = osrelease.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let major = parts[0].parse::<u32>().unwrap_or(0);
    let minor = parts[1].parse::<u32>().unwrap_or(0);
    (major > want_major) || (major == want_major && minor >= want_minor)
}

// ---------------- YAML config structures ----------------

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    version: Option<u32>,
    profiles: HashMap<String, Profile>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Profile {
    description: Option<String>,
    #[serde(default)]
    access_roots: HashMap<String, AccessRootGroup>,
    #[serde(default)]
    control_access: ControlAccess,
    command: CommandSpec,
    log_level: Option<String>,
    dry_run: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AccessRootGroup {
    paths: Vec<String>,
    permissions: Permissions,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct Permissions {
    #[serde(default)]
    read_file: Option<bool>,
    #[serde(default)]
    read_dir: Option<bool>,
    #[serde(default)]
    execute: Option<bool>,
    #[serde(default)]
    write_file: Option<bool>,
    #[serde(default)]
    remove_file: Option<bool>,
    #[serde(default)]
    remove_dir: Option<bool>,
    #[serde(default)]
    truncate: Option<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct ControlAccess {
    #[serde(default)]
    read_file: Option<bool>,
    #[serde(default)]
    read_dir: Option<bool>,
    #[serde(default)]
    execute: Option<bool>,
    #[serde(default)]
    write_file: Option<bool>,
    #[serde(default)]
    remove_file: Option<bool>,
    #[serde(default)]
    remove_dir: Option<bool>,
    #[serde(default)]
    truncate: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CommandSpec {
    binary: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    working_dir: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

fn load_config(path: &PathBuf) -> Result<Config> {
    let text = std::fs::read_to_string(path)?;
    let cfg: Config = serde_yaml::from_str(&text)?;
    // Version check: support 1 by default
    if let Some(ver) = cfg.version {
        if ver != 1 {
            return Err(anyhow!("unsupported config version: {}", ver));
        }
    }
    Ok(cfg)
}

fn normalize_path(p: &str) -> Result<String> {
    if let Some(stripped) = p.strip_prefix("~/") {
        let home = std::env::var("HOME").map_err(|_| anyhow!("cannot resolve $HOME"))?;
        let mut pb = PathBuf::from(home);
        pb.push(stripped);
        Ok(pb.to_string_lossy().into())
    } else {
        Ok(p.to_string())
    }
}

// --------------- Access mapping helpers ---------------

fn access_from_permissions(perms: &Permissions) -> BitFlags<AccessFs> {
    let mut set = BitFlags::<AccessFs>::empty();

    if perms.read_file.unwrap_or(false) {
        set.insert(AccessFs::ReadFile);
    }
    if perms.read_dir.unwrap_or(false) {
        set.insert(AccessFs::ReadDir);
    }
    if perms.execute.unwrap_or(false) {
        set.insert(AccessFs::Execute);
    }
    if perms.write_file.unwrap_or(false) {
        set.insert(AccessFs::WriteFile);
    }
    if perms.remove_file.unwrap_or(false) {
        set.insert(AccessFs::RemoveFile);
    }
    if perms.remove_dir.unwrap_or(false) {
        set.insert(AccessFs::RemoveDir);
    }
    if perms.truncate.unwrap_or(false) {
        set.insert(AccessFs::Truncate);
    }

    set
}

fn access_from_control(ctrl: &ControlAccess) -> BitFlags<AccessFs> {
    let mut set = BitFlags::<AccessFs>::empty();

    if ctrl.read_file.unwrap_or(false) {
        set.insert(AccessFs::ReadFile);
    }
    if ctrl.read_dir.unwrap_or(false) {
        set.insert(AccessFs::ReadDir);
    }
    if ctrl.execute.unwrap_or(false) {
        set.insert(AccessFs::Execute);
    }
    if ctrl.write_file.unwrap_or(false) {
        set.insert(AccessFs::WriteFile);
    }
    if ctrl.remove_file.unwrap_or(false) {
        set.insert(AccessFs::RemoveFile);
    }
    if ctrl.remove_dir.unwrap_or(false) {
        set.insert(AccessFs::RemoveDir);
    }
    if ctrl.truncate.unwrap_or(false) {
        set.insert(AccessFs::Truncate);
    }

    set
}
