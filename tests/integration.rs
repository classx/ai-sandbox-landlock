use assert_cmd::Command;
use predicates::str::contains;
use serde_yaml::Value;

fn bin_cmd() -> Command {
    assert_cmd::cargo::cargo_bin_cmd!("ai-sandbox-landlock")
}

#[test]
fn print_ruleset_minimal() {
    let mut cmd = bin_cmd();
    cmd.arg("--config")
        .arg("examples/ai-sandbox-landlock.yaml")
        .arg("--profile")
        .arg("minimal")
        .arg("--print-ruleset");
    cmd.assert()
        .success()
        .stdout(contains("Ruleset (profile mode)"))
        .stdout(contains("groups:"))
        .stdout(contains("projects:"))
        .stdout(contains("allowed"));
}

#[test]
fn dry_run_minimal() {
    let mut cmd = bin_cmd();
    cmd.arg("--config")
        .arg("examples/ai-sandbox-landlock.yaml")
        .arg("--profile")
        .arg("minimal")
        .arg("--dry-run");
    cmd.assert()
        .success()
        .stdout(contains("Ruleset (profile mode)"));
}

#[test]
fn print_config_minimal() {
    let mut cmd = bin_cmd();
    cmd.arg("--config")
        .arg("examples/ai-sandbox-landlock.yaml")
        .arg("--profile")
        .arg("minimal")
        .arg("--print-config");
    cmd.assert()
        .success()
        .stdout(contains("access_roots"))
        .stdout(contains("command:"));
}

#[test]
fn root_mode_print_ruleset() {
    let mut cmd = bin_cmd();
    cmd.arg("--root")
        .arg("/usr")
        .arg("--read-only")
        .arg("--print-ruleset");
    cmd.assert()
        .success()
        .stdout(contains("Ruleset (root mode)"))
        .stdout(contains("allowed"));
}

#[test]
fn generate_profile_output_structure() {
    let tmp_root = "/tmp/ai-sandbox-integ-root";

    let mut cmd = bin_cmd();
    cmd.arg("--generate-profile")
        .arg("--gen-name")
        .arg("integ")
        .arg("--root")
        .arg(tmp_root);

    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");

    let doc: Value = serde_yaml::from_str(&stdout).expect("valid YAML output");

    // Validate version
    assert_eq!(doc.get("version").and_then(|v| v.as_i64()), Some(1));

    // Validate profiles mapping and target profile
    let profiles = doc
        .get("profiles")
        .and_then(|v| v.as_mapping())
        .expect("profiles must be a mapping");

    let profile_key = Value::String("integ".to_string());
    let profile = profiles
        .get(&profile_key)
        .expect("generated profile 'integ' must exist")
        .as_mapping()
        .expect("profile must be a mapping");

    // access_roots contains system, cache, projects
    let access_roots = profile
        .get(Value::String("access_roots".into()))
        .and_then(|v| v.as_mapping())
        .expect("access_roots must be a mapping");

    for key in ["system", "cache", "projects"] {
        assert!(
            access_roots.get(Value::String(key.into())).is_some(),
            "missing access_roots group: {}",
            key
        );
    }

    // command binary is /bin/bash and working_dir equals tmp_root
    let command = profile
        .get(Value::String("command".into()))
        .and_then(|v| v.as_mapping())
        .expect("command must be a mapping");

    let binary = command
        .get(Value::String("binary".into()))
        .and_then(|v| v.as_str())
        .expect("command.binary must be a string");
    assert_eq!(binary, "/bin/bash");

    let working_dir = command
        .get(Value::String("working_dir".into()))
        .and_then(|v| v.as_str())
        .expect("command.working_dir must be a string");
    assert_eq!(working_dir, tmp_root);

    // projects.paths contains tmp_root
    let projects = access_roots
        .get(Value::String("projects".into()))
        .and_then(|v| v.as_mapping())
        .expect("projects must be a mapping");
    let paths = projects
        .get(Value::String("paths".into()))
        .and_then(|v| v.as_sequence())
        .expect("projects.paths must be a sequence");
    let contains_root = paths.iter().any(|p| p.as_str() == Some(tmp_root));
    assert!(
        contains_root,
        "projects.paths should contain the provided root"
    );
}
