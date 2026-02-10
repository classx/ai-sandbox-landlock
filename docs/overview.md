# ai-sandbox-landlock — Project Overview

This project is a minimal launcher (prototype) that runs a given command in a restricted environment using the Linux Landlock LSM. It supports two operation modes:
- Profile mode: filesystem restrictions and permissions described in a YAML profile.
- Root mode: restrict filesystem access to a single project root directory.

Focus areas:
- Explicit control of file/dir rights: ReadFile, ReadDir, Execute, WriteFile, RemoveFile, RemoveDir, Truncate.
- ABI-aware diagnostics: shows which rights will be ignored when the current kernel ABI does not support them.
- Usability: CLI flags for capability checks, printing rules, dry-run, strict mode, log level, and profile generation.

## Requirements
- Linux kernel ≥ 5.13 and `landlock` present in `/sys/kernel/security/lsm` (to apply restrictions).
- Rust toolchain to build.

## Installation
- Build:
  - `cargo build --release`
- Install binary:
  - `make install` (installs to `~/.local/bin/ai-sandbox-landlock`)

## Quick Start
- Restrict to `/usr` and allow only read/execute; print rules without enforcing:
  - `ai-sandbox-landlock --root /usr --read-only --print-ruleset`
- Run a command inside the sandbox (if Landlock available):
  - `ai-sandbox-landlock --root /path/to/project -- echo "hello"`
- Check Landlock availability:
  - `ai-sandbox-landlock --check`

## YAML Profiles
Schema (simplified):
- `version`: schema version (supports `1`).
- `profiles`: mapping of profile name → profile.
  - `description`: profile description.
  - `access_roots`: groups of paths with permissions:
    - `paths`: list of paths.
    - `permissions`: rights (`read_file`, `read_dir`, `execute`, `write_file`, `remove_file`, `remove_dir`, `truncate`).
  - `control_access`: global rights to handle by the ruleset (e.g., enable `execute`).
  - `command`: what to run inside the sandbox (`binary`, `args`, `working_dir`, `env`).
  - `log_level`: logging level.
  - `dry_run`: print rules without execution.

See example: [private/projects/ai-sandbox-landlock/examples/ai-sandbox-landlock.yaml](private/projects/ai-sandbox-landlock/examples/ai-sandbox-landlock.yaml)

## Diagnostics and ABI
- Maximum supported ABI is probed dynamically; if a right is unsupported, it appears under "ignored".
- Printing modes:
  - `--print-config`: show the selected profile or current parameters.
  - `--print-ruleset`: print the assembled ruleset (no enforcement).
  - `--dry-run`: same as `--print-ruleset`, then exit.

## Security
- Landlock only tightens access; it does not undo DAC/SELinux/AppArmor.
- Symlinks, previously opened file descriptors, and bind mounts may affect isolation.
Details: [private/projects/ai-sandbox-landlock/SECURITY.md](private/projects/ai-sandbox-landlock/SECURITY.md)

## Profile Generation
- `--generate-profile` creates a YAML profile automatically:
  - Detects project root via `git rev-parse --show-toplevel` or `--root`.
  - Produces `system`, `cache`, and `projects` groups, and a `/bin/bash` command.
  - Writes to a file via `--output` or prints to stdout.

Examples:
- `ai-sandbox-landlock --generate-profile --gen-name myproj`
- `ai-sandbox-landlock --generate-profile --root ~/work/app --gen-name app --output app.yaml`
