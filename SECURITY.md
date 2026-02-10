# ai-sandbox-landlock — Security Notes and Edge Cases

## Overview
This document captures practical behaviors, limitations, and recommendations when using Landlock to sandbox developer tools (IDE, LLM backends). Landlock tightens access; it never grants permissions beyond DAC/other LSMs.

## Symlinks
- Resolution happens at operation time. If a symlink resolves outside allowed trees, the operation is denied according to handled rights.
- Profiles should prefer real directory paths. Avoid placing symlinks inside allowed roots that point out of those roots.
- For caches or temp storage, use dedicated subdirectories within explicitly allowed paths.

## Bind Mounts
- Bind mounts can expose external trees under allowed paths. Landlock restricts based on the filesystem hierarchy as seen in the current mount namespace.
- Recommendations:
  - Combine Landlock with a mount namespace to hide or remap filesystem trees as needed.
  - Avoid using bind mounts inside allowed roots unless you fully trust their targets.

## Open File Descriptors (FDs)
- Landlock applies at `restrict_self` time. Already-open FDs retain their capabilities.
- Launcher guidelines:
  - Do not open broad FDs (e.g., directories like `/` or `/home`) before applying restrictions.
  - Close configuration and probe file handles before `restrict_self`. The current launcher only opens config and probe files, which are closed beforehand.

## Inheritance and Process Behavior
- Restrictions are inherited by child processes.
- The launcher applies `restrict_self` before executing the target command; subsequent execs in the target process inherit restrictions.

## Coexistence with DAC/SELinux/AppArmor
- Landlock only tightens access beyond DAC/LSMs; it cannot override their denials nor grant extra rights.
- Expect combined effects: an operation must be permitted by DAC/LSMs and not denied by Landlock.

## ABI and Supported Rights
- Supported rights depend on the kernel’s Landlock ABI version.
- The launcher detects the maximum supported ABI and computes the supported rights. In `--dry-run` and `--print-ruleset`, any requested but unsupported rights are listed as ignored.

## Recommendations
- Use absolute, real paths in profiles; avoid symlinked paths that escape allowed trees.
- Prefer dedicated cache directories and minimal allowed roots.
- Consider mount namespaces for stronger isolation of filesystem layout.
- Keep `restrict_self` as early as practical; avoid opening unnecessary FDs before it.
- Use `--require-landlock` when sandboxing is mandatory; otherwise, the launcher warns and proceeds un-sandboxed.

## Known Limitations
- Landlock does not hide directories; it denies operations. Listing may still show entries, but access operations are blocked.
- Operations with file capabilities or privileged behavior are still subject to Landlock’s denials (Landlock does not add privileges).
- Some rights (e.g., Truncate) may be unsupported on older ABIs and thus ignored; the launcher reports these in diagnostics.
