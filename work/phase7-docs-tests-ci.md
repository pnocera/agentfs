# Phase 7 - Docs, Tests, And CI

Date: 2026-05-26

## Scope

Phase 7 makes the Windows port maintainable after the core mount/run and
secondary CLI work.

Implemented:

- Documented Windows requirements in `MANUAL.md`.
- Updated `README.md` to include Windows NFS drive-letter mounts and the
  Windows v1 no-sandbox limitation for `agentfs run`.
- Added Windows verification and manual smoke guidance to `TESTING.md`.
- Added `.github/workflows/windows-rust.yml` with Windows CLI and Rust SDK jobs.
- The CLI CI job installs nightly explicitly, matching `cli/rust-toolchain.toml`.
- Updated the README build-status badge to point at `windows-rust.yml`.
- Updated `agentfs serve nfs` Windows startup examples to include both cmd.exe
  and PowerShell forms, using the same `timeout=8,retry=1` options as the
  runtime mount probe.
- Added a code comment documenting the `.db` path heuristic for NFS server
  arguments.
- Simplified `resolve_db_path` after the `.agentfs/<id>.db` path is built; the
  database is created later if it does not exist.

## CI Boundary

The Windows workflow runs checks that do not require kernel-driver or optional
feature installation:

- CLI formatting, default check, no-default check, clippy, library tests, and
  binary build on `windows-latest`.
- Rust SDK formatting, check, and test on `windows-latest`.

The CLI job uses `dtolnay/rust-toolchain@nightly` because the CLI crate pins
nightly in `cli/rust-toolchain.toml`. The SDK job uses stable.

It intentionally does not run a live `agentfs mount` or `agentfs run` smoke,
because those require the Windows Client for NFS optional feature and a free
local portmapper port. Live mount acceptance remains manual and
environment-dependent.

## Existing Unit Coverage

Phase 7 relies on the tests added during the Windows port phases:

- Drive-letter mountpoint parser and Windows NFS source forms:
  `cli/src/mount/nfs.rs`.
- Windows NFS server printed source forms:
  `cli/src/cmd/nfs.rs`.
- NFS device major/minor helpers:
  `cli/src/nfs.rs` and `sdk/rust/src/filesystem/mod.rs`.
- Windows HostFS direct operations:
  `sdk/rust/src/filesystem/hostfs_windows.rs`.
- Windows overlay copy-on-write behavior:
  `sdk/rust/src/filesystem/overlayfs.rs`.
- Windows `agentfs run` drive-letter and shell-preparation helpers:
  `cli/src/cmd/run_windows.rs`.

## Verification

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo fmt --manifest-path cli\Cargo.toml -- --check` | PASS | `work/phase7-cargo-fmt.log` |
| `cargo fmt --manifest-path sdk\rust\Cargo.toml -- --check` | PASS | `work/phase7-cargo-fmt.log` |
| CLI `cargo check --target x86_64-pc-windows-msvc` | PASS | `work/phase7-cargo-check-windows-cli-default-features.log` |
| CLI `cargo check --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase7-cargo-check-windows-cli-no-default-features.log` |
| CLI `cargo clippy --target x86_64-pc-windows-msvc --all-targets` | PASS with pre-existing warnings in `init.rs` and `ps.rs` | `work/phase7-cargo-clippy-windows-cli-all-targets.log` |
| CLI `cargo test --target x86_64-pc-windows-msvc --lib` | PASS, 44 tests | `work/phase7-cargo-test-windows-cli-lib.log` |
| CLI `cargo build --target x86_64-pc-windows-msvc` | PASS | `work/phase7-cargo-build-windows-cli.log` |
| SDK `cargo test --target x86_64-pc-windows-msvc` | PASS, 76 tests plus 1 doc-test | `work/phase7-cargo-test-windows-sdk.log` |
| Windows standalone NFS server startup smoke | PASS | `work/phase7-cli-smoke-windows-serve-nfs.log` |
| Linux no-default CLI check | PASS | `work/phase7-cargo-check-linux-cli-no-default-features.log` |
| macOS default CLI check | PASS | `work/phase7-cargo-check-macos-cli-default-features.log` |
| `git diff --check` | PASS | command completed successfully |

## Manual Smoke Status

The documented manual smoke remains:

```powershell
agentfs init win-direct
agentfs fs win-direct write /hello.txt hello
agentfs mount win-direct Z: --backend nfs -f
Get-Content Z:\hello.txt
Set-Content Z:\created.txt created
& $env:SystemRoot\System32\umount.exe Z:
agentfs fs win-direct cat /created.txt
agentfs run --session win-run cmd.exe /c "echo hello>created.txt"
agentfs diff win-run
```

The command sequence is documented as a two-terminal flow because the foreground
NFS mount process must remain running while the drive is accessed.

The manual also calls out that `agentfs mount` starts its internal Windows NFS
server at port 111 when available, while `agentfs serve nfs` examples may use an
explicitly selected high port such as 11111.
