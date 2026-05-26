# Phase 5 - Windows Run V1

Date: 2026-05-26

## Scope

Phase 5 replaces the Windows `agentfs run` stub with overlay-only copy-on-write execution. Windows v1 does not provide an OS security sandbox; the child process receives a mounted copy-on-write working directory and explicit environment markers.

## Changed Files

- `cli/src/cmd/run_windows.rs`
- `cli/src/cmd/run.rs`
- `cli/src/main.rs`
- `cli/src/opts.rs`

## Implementation Notes

- Replaced the Windows `run` stub with a real implementation.
- Builds the run session under `%USERPROFILE%\.agentfs\run\<session-id>`.
- Opens or creates the session delta database at `delta.db`.
- Creates `HostFS` over the current working directory and layers it under `OverlayFS`.
- Initializes overlay metadata and writes the base path to `base_path`.
- Selects an unused drive letter from `Z:\` down to `D:\`.
- Mounts the overlay through `crate::mount::mount_fs(..., MountBackend::Nfs)`.
- Runs the child with current directory set to the mounted drive root.
- Sets:
  - `AGENTFS=1`
  - `AGENTFS_SANDBOX=windows-overlay-only`
  - `AGENTFS_SESSION=<session-id>`
- Updates the default shell on Windows to `cmd.exe`.
- Adds prompt setup for interactive default shells:
  - `cmd.exe` gets `/K prompt [agentfs] $P$G`.
  - `powershell.exe` / `pwsh.exe` get a `prompt` function through `-NoExit -Command`.
  - Explicit shell commands with arguments are left unchanged.
- Handles unsupported or non-enforceable flags explicitly:
  - `--experimental-sandbox` errors on Windows.
  - `--strace` errors on Windows.
  - `--allow`, `--no-default-allows`, and `--system` warn and are ignored on Windows v1.
- Uses `tokio::signal::ctrl_c` while waiting for the child so Ctrl+C can terminate the child and then drop the mount handle for normal cleanup.
- Updates run help text to state that Windows v1 is overlay-only and not a security sandbox.

## Acceptance Mapping

- The Windows run command now compiles into the default-feature Windows CLI.
- Unit coverage verifies drive-letter selection and shell prompt argument handling.
- The CLI help text states the Windows no-sandbox contract and ignored Windows flags.
- Runtime smoke reaches the Windows NFS mount path, then fails with the same Windows Client for NFS `Network Error - 53` seen in Phases 3 and 4. Because the drive never mounts, the child command is not started and the full "write into delta, not base" acceptance test remains blocked by the pre-existing NFS client/runtime issue.

## Verification

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo check --target x86_64-pc-windows-msvc` in `cli` | PASS | `work/phase5-cargo-check-windows-cli-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --lib` in `cli` | PASS, 41 tests | `work/phase5-cargo-test-windows-cli-lib-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc` in `cli` | PASS with two pre-existing warnings in `init.rs` and `ps.rs` | `work/phase5-cargo-clippy-windows-cli-default-features.log` |
| `cargo build --target x86_64-pc-windows-msvc` in `cli` | PASS | `work/phase5-cargo-build-windows-cli-default-features.log` |
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` in `cli` | PASS | `work/phase5-cargo-check-windows-cli-no-default-features.log` |
| `agentfs.exe run --help` | PASS, documents Windows overlay-only/no-sandbox behavior | `work/phase5-cli-help-run-windows.log` |
| `agentfs.exe run --session phase5-run-smoke cmd.exe /c "echo hello>created.txt"` | BLOCKED by known Windows Client for NFS `Network Error - 53`; base file was not created | `work/phase5-cli-smoke-windows-run.log` |
| `cargo check --target x86_64-unknown-linux-gnu --no-default-features` in `cli` via WSL | PASS | `work/phase5-cargo-check-linux-cli-no-default-features.log` |
| `cargo check --target x86_64-unknown-linux-gnu` in `cli` via WSL | FAIL, environment missing `libunwind-ptrace` for the Linux sandbox stack | `work/phase5-cargo-check-linux-cli-default-features.log` |
| `cargo check --target x86_64-apple-darwin --no-default-features` in `cli` | PASS | `work/phase5-cargo-check-macos-cli-no-default-features.log` |
| `cargo check --target x86_64-apple-darwin` in `cli` | PASS | `work/phase5-cargo-check-macos-cli-default-features.log` |
| `git diff --check` | PASS | command completed with exit code 0 |

## Known Blockers

- Windows run cannot complete live execution until the Windows NFS client can mount the localhost AgentFS NFS export. The current machine still returns `Network Error - 53` for all supported localhost forms.
- Linux default-feature WSL check requires the system package that provides `libunwind-ptrace.pc`; the no-default Linux check passes, and this is unrelated to the Windows run code.
