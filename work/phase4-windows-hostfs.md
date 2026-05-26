# Phase 4 - Windows HostFS

Date: 2026-05-26

## Scope

Phase 4 implements a Windows `HostFS` for `agentfs-sdk` and removes the Phase 3 overlay-mount limitation on Windows. This unlocks overlay-backed NFS mount setup and provides the filesystem substrate needed by the later Windows `agentfs run` phase.

## Changed Files

- `sdk/rust/Cargo.toml`
- `sdk/rust/Cargo.lock`
- `cli/Cargo.lock`
- `sandbox/Cargo.lock`
- `sdk/rust/src/filesystem/hostfs_windows.rs`
- `sdk/rust/src/filesystem/mod.rs`
- `sdk/rust/src/lib.rs`
- `sdk/rust/src/filesystem/overlayfs.rs`
- `cli/src/cmd/mount.rs`

## Implementation Notes

- Added a Windows-only SDK dependency on `windows-sys` for file identity, file time, and volume space APIs.
- Added `sdk/rust/src/filesystem/hostfs_windows.rs`.
- Re-exported `HostFS` on Windows from the SDK filesystem module and SDK crate root.
- Implemented path-backed Windows `HostFS` operations for:
  - `lookup`, `getattr`, `readlink`, `readdir`, `readdir_plus`
  - `open`, `mkdir`, `create_file`, `mknod` for regular files
  - `unlink`, `rmdir`, `rename`, `chmod`, `utimens`, `statfs`
  - `link` and `symlink` with native Windows behavior
- Implemented explicit unsupported errors for Windows-only gaps that should not silently fake success:
  - `chown`
  - `mknod` for non-regular special files
- Synthesized POSIX-style metadata from Windows metadata:
  - file type bits for directory, regular file, and symlink
  - readonly-bit-aware permissions
  - zero uid/gid and `rdev`
  - Windows FILETIME conversion for atime, mtime, and ctime
- Implemented stable inode mapping:
  - preferred identity source: `GetFileInformationByHandleEx(FileIdInfo)` using volume serial plus `FILE_ID_128`
  - fallback identity source: `GetFileInformationByHandle` using volume serial plus file index
  - deterministic 63-bit FNV-1a fingerprint into `Stats.ino`
  - inode `1` reserved for root
  - per-`HostFS` bidirectional cache detects collisions and probes to a deterministic available inode value for the current process
- Documented by implementation shape: the mapping is stable for normal Windows file identities and collision-safe within a `HostFS` instance. A future cross-process collision registry should be added if persistent cross-process inode collision handling becomes a requirement.
- Tightened Windows rename behavior after local review:
  - verifies the source before touching an existing destination
  - preserves destination entries on missing-source and type-mismatch failures
  - handles file overwrite on Windows, where `std::fs::rename` cannot replace an existing file
  - removes stale overwritten destination inodes from the per-process cache
- Updated Windows overlay tests for copy-on-write file behavior and create-file-in-base-directory behavior.
- Updated `agentfs mount` overlay flow so Windows now builds `HostFS` plus `OverlayFS` instead of returning the Phase 3 "Windows HostFS required" error.

## Acceptance Mapping

- `agentfs-sdk` direct filesystem tests pass on Windows.
- Windows HostFS tests cover basic file I/O, directory listing, stable identity, create/truncate, mutations, overwrite rename, type-mismatch rename preservation, and missing-source rename preservation.
- Windows overlay tests cover copy-on-write and creating a delta file under a base directory.
- Overlay-backed `agentfs mount ... --backend nfs -f` no longer stops at the Phase 3 HostFS error. It now constructs the overlay and reaches the Windows NFS client.
- Live Windows NFS mounting still fails with the previously documented Windows Client for NFS `Network Error - 53` for localhost export forms. This remains the Phase 0-3 client/runtime issue, not a Phase 4 HostFS blocker.

## Verification

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo check --target x86_64-pc-windows-msvc` in `sdk/rust` | PASS | `work/phase4-cargo-check-windows-sdk.log` |
| `cargo test --target x86_64-pc-windows-msvc` in `sdk/rust` | PASS, 76 tests plus 1 doc-test | `work/phase4-cargo-test-windows-sdk.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --lib --tests` in `sdk/rust` | PASS | `work/phase4-cargo-clippy-windows-sdk-lib-tests.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --all-targets` in `sdk/rust` | FAIL, pre-existing benchmark API drift | `work/phase4-cargo-clippy-windows-sdk.log` |
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` in `cli` | PASS | `work/phase4-cargo-check-windows-cli-no-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --no-default-features` in `cli` | PASS with the three pre-existing warnings from `init.rs`, `ps.rs`, and `run_not_supported.rs` | `work/phase4-cargo-clippy-windows-cli-no-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --no-default-features mount` in `cli` | PASS, 8 tests | `work/phase4-cargo-test-windows-cli-mount.log` |
| `cargo check --target x86_64-pc-windows-msvc` in `cli` | PASS | `work/phase4-cargo-check-windows-cli-default-features.log` |
| `cargo build --target x86_64-pc-windows-msvc --no-default-features` in `cli` | PASS | `work/phase4-cargo-build-windows-cli-no-default-features.log` |
| Overlay-backed Windows mount smoke | Reaches overlay setup and Windows NFS client, then fails with known `Network Error - 53` | `work/phase4-cli-smoke-windows-overlay-mount-nfs.log` |
| `cargo check --target x86_64-unknown-linux-gnu` in `sdk/rust` | PASS | `work/phase4-cargo-check-linux-sdk.log` |
| `cargo check --target x86_64-unknown-linux-gnu --no-default-features` in `cli` | PASS | `work/phase4-cargo-check-linux-cli-no-default-features.log` |
| `cargo check --target x86_64-apple-darwin` in `sdk/rust` | PASS | `work/phase4-cargo-check-macos-sdk.log` |
| `cargo check --target x86_64-apple-darwin --no-default-features` in `cli` | PASS | `work/phase4-cargo-check-macos-cli-no-default-features.log` |
| `git diff --check` | PASS | command completed with exit code 0 |

## Known Non-Blocking Issue

`sdk/rust` all-target clippy still fails because benchmark targets use stale `OverlayFS` APIs:

- `benches/overlayfs.rs` imports an unused `FileSystem` symbol and calls missing `OverlayFS::remove`.
- `benches/workload.rs` still calls old path-based `OverlayFS` methods such as `create_file(path, ...)`, `lstat`, `mkdir(path, ...)`, `open(path)`, `readdir_plus(path)`, and `stat`.

The focused SDK `--lib --tests` clippy run passes and covers the Phase 4 implementation and tests.
