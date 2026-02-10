# ai-sandbox-landlock

A minimal Rust launcher that applies Linux Landlock LSM restrictions to developer tools (IDE, Copilot backends, local LLMs) using declarative YAML profiles. Runs as an unprivileged user and sandboxes itself before executing a target command.

## Requirements
- Linux kernel ≥ 5.13 with Landlock enabled (lsm includes `landlock`).
- Rust toolchain (cargo) for building.

## Install
Local project build and run:

```bash
# Build
git clone git@github.com:classx/ai-sandbox-landlock.git
cd ai-sandbox-landlock
cargo build

# Run capability check
cargo run -- --check
```

Optional local install (unpublished):

```bash
# Install locally (path)
cargo install --path .
# Or copy built binary
install -Dm755 target/release/ai-sandbox-landlock ~/.local/bin/ai-sandbox-landlock
```

## Usage
Two modes: profile-based and root-only.

Profile-based:
```bash
# Print rules for a profile
ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile minimal --print-ruleset

# Dry-run a profile (no enforcement, no exec)
ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile vscode-copilot --dry-run

# Enforce sandbox then run command from profile
ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile vscode-copilot

# Override command in profile
ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile vscode-copilot -- /usr/bin/echo ok
```

Root-only:
```bash
# Allow read-only under /usr and execute a command
ai-sandbox-landlock --root /usr --read-only -- /usr/bin/echo ok

# Print rules for the root-only mode
ai-sandbox-landlock --root /usr --read-only --print-ruleset
```

Common flags:
- `--dry-run`: Print planned rules; no enforcement, no exec.
- `--print-ruleset`: Print handled rights and per-path rules, then exit.
- `--print-config`: Dump selected profile YAML, then exit.
- `--require-landlock`: Fail if Landlock is unavailable; otherwise warn and run unsandboxed.
- `--log-level {error|warn|info|debug|trace}`: Set logging verbosity.

Generate a profile (dynamic):
```bash
# Generate YAML for the current git project to stdout
ai-sandbox-landlock --generate-profile --gen-name myproj

# Generate using an explicit root and save to a file
ai-sandbox-landlock --generate-profile --root ~/dev/myproj --gen-name myproj --output myproj.yaml
```

## YAML Schema
Profiles file structure (simplified):
- `version`: schema version (supports `1`).
- `profiles.<name>`:
  - `description`: optional.
  - `access_roots.<group>.paths`: array of path strings.
  - `access_roots.<group>.permissions`: booleans for rights (`read_file`, `read_dir`, `execute`, `write_file`, `remove_file`, `remove_dir`, `truncate`).
  - `control_access`: global handled rights for the ruleset.
  - `command`: `binary`, `args`, `working_dir`, `env`.
  - `log_level`, `dry_run`: optional defaults per profile.

See example: [examples/ai-sandbox-landlock.yaml](examples/ai-sandbox-landlock.yaml).

## Security Notes
- Landlock tightens access; it doesn’t grant privileges beyond DAC/SELinux/AppArmor.
- Restrictions apply at `restrict_self`; already-open FDs retain their capabilities.
- Symlink targets resolving outside allowed trees are denied; avoid using symlinks in allowed paths.
- Bind mounts can expose external trees; consider combining with a mount namespace.
- The launcher detects the maximum supported ABI and reports unsupported rights (ignored) in `--dry-run`/`--print-ruleset`.

Details: [SECURITY.md](SECURITY.md).

## VSCode Integration (example)
Wrap VSCode with the launcher for a project:
```bash
# Example alias (adjust paths)
alias code-sandbox='ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile vscode-copilot --'
# Run inside a project directory
code-sandbox code .
```

## Troubleshooting
- Landlock unavailable: `--check` reports kernel and LSM status; use `--require-landlock` to enforce.
- Command fails with `Permission denied`: the profile likely lacks required rights or paths; use `--dry-run` to inspect rules.
- Missing system paths for execution: include `/usr`, `/lib`, `/lib64` with `execute` rights in a system group.
- `truncate` ignored: older ABIs don’t support it; it will be listed under ignored rights.

## Development
Run tests:
```bash
cargo test
```

Format/lint:
```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```


---
Non-commercial use only. Contact for commercial licensing.
