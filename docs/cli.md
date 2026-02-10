# CLI Parameters

Below are the flags and arguments of `ai-sandbox-landlock` with detailed descriptions and available options.

- `--config <PATH>`: path to a YAML configuration file.
  - Requires `--profile`.
  - Supports schema version `1`.

- `--profile <NAME>`: profile name to load from `--config`.
  - Required when using `--config`.

- `--root <PATH>`: project root for root mode.
  - In profile mode it is optional; may be inferred from `access_roots.projects`.

- `--read-only`: enable read-only mode for `--root`.
  - Allows `Execute` within the root.
  - Default: `false`.

- `--check`: check Landlock availability and exit.
  - Prints a report: kernel version, presence in LSM list.

- `--dry-run`: build and print rules without enforcing or running.
  - Useful for debugging profiles.

- `--require-landlock`: fail if Landlock is not available.
  - Default: `false`.

- `--log-level <LEVEL>`: logging level.
  - Accepted: `error`, `warn`, `info`, `debug`, `trace`.
  - Default: `info`.

- `--no-color`: disable ANSI-colored logs.
  - Useful for CI/pipes and terminals without color support.

- `--print-config`: print the selected profile or current parameters and exit.

- `--print-ruleset`: print the assembled ruleset and exit.

- `--generate-profile`: generate a YAML profile and exit.
  - Detects the root via `git` or `--root`.
  - Produces `system`, `cache`, `projects` groups and a `/bin/bash` command.

- `--gen-name <NAME>`: the name of the generated profile.
  - Default — basename of the project root.

- `--output <PATH>`: path to save the generated YAML.
  - If omitted, prints to stdout.

- `-- <CMD> [ARGS...]`: command to run inside the sandbox.
  - In profile mode, the command can be set via `command.binary`/`command.args`.

## Behavior and Precedence
- With `--config`/`--profile`, values from YAML may override some CLI parameters:
  - `command` from the profile is used if no tail `--` is provided.
  - `root` is taken from `access_roots.projects[0]` if not set via CLI.
  - `read_only` may be inferred from `permissions` of the `projects` group (no write/remove/truncate).
  - `log_level` is taken from the profile if not set via CLI.

## Diagnostics and Output
- `--print-ruleset` and `--dry-run` show:
  - `handled`: union of all rights handled by the ruleset.
  - `ignored`: rights ignored due to ABI limitations.
  - List of groups and paths with their allowed rights.

## Examples
- Print rules for root mode:
  - `ai-sandbox-landlock --root /usr --read-only --print-ruleset`
- Print a profile:
  - `ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile minimal --print-config`
- Dry-run a profile:
  - `ai-sandbox-landlock --config examples/ai-sandbox-landlock.yaml --profile minimal --dry-run`
- Generate a profile into a file:
  - `ai-sandbox-landlock --generate-profile --gen-name myproj --output myproj.yaml`
