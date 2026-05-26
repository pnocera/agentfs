# AgentFS Windows Port Implementation Plan

Date: 2026-05-26
Source report: `work/windows-port-report.md`
Repo: `F:\Tools\agentfs`

## Goal

Ship a Windows port in small, verifiable slices:

1. `agentfs mount <id> <drive> --backend nfs` works on Windows for direct AgentFS databases.
2. Overlay-backed mounts and `agentfs run` work after a Windows `HostFS` implementation exists.
3. Windows errors are explicit when a user asks for unsupported behavior such as FUSE or sandbox isolation.
4. Linux and macOS behavior remains unchanged.

## Scope Decisions

- Use NFS for Windows v1. Do not attempt WinFsp/FUSE in this pass.
- Treat Windows mountpoints as unassigned drive letters such as `Z:` or `Z:\`.
- Use the built-in Windows Client for NFS first. WinFsp-NFS is a later fallback, not a v1 dependency.
- `run` v1 has no Windows OS sandbox. It may provide copy-on-write working-directory behavior, but it must not claim filesystem confinement.
- Defer AppContainer, Job Object confinement, WinFsp drive mounts, and directory junction exposure to v2.

## Current Checkout Facts

- `cli/src/cmd/mod.rs` compiles `mount_stub.rs` on non-Unix targets, so Windows never reaches backend dispatch.
- `cli/src/cmd/run_windows.rs` exists but only bails.
- `cli/src/lib.rs` gates `nfsserve`, `nfs`, and `mount` behind `#[cfg(unix)]`; these gates must change before the NFS path can compile on Windows.
- `cli/src/nfs.rs` imports `libc::{O_RDONLY, O_RDWR}` and uses `libc::{major, minor, makedev}`. That must be made target-independent or Windows-gated.
- `agentfs_sdk::HostFS` is only re-exported on Linux/macOS. Direct AgentFS NFS mounts can land before this, but overlay-backed mounts and `run` need a Windows `HostFS`.
- `rg fs_util cli/src` shows no call sites beyond the `nfsserve::mod` declaration, so `nfsserve::fs_util` is not an immediate mount/run blocker.

## Phase 0 - Baseline And Client Spike

Purpose: establish the current Windows compile surface and remove uncertainty around the Windows NFS client.

Tasks:

- Run and save baseline checks:
  - `cargo check -p agentfs --target x86_64-pc-windows-msvc --no-default-features`
  - `cargo check -p agentfs --target x86_64-pc-windows-msvc`
- Verify the exact Windows Client for NFS command syntax for:
  - localhost export path
  - non-default NFS port
  - drive-letter target
  - unmount command
- Decide the exact preflight detection strategy:
  - Prefer `%SystemRoot%\System32\mount.exe` / `%SystemRoot%\System32\umount.exe`.
  - Reject Git Bash, MSYS, Cygwin, or other unrelated `mount.exe` binaries.
  - Return an actionable error if Client for NFS is missing.

Acceptance:

- Baseline logs identify only expected gates/stubs or known Windows blockers.
- A manual NFS client command has been confirmed before implementation hardcodes the UNC/port form.
- The exact working command line is saved to `work/windows-nfs-client-spike.md`, including whether the non-default port form is `\\127.0.0.1@PORT\!`, `\\127.0.0.1:PORT\!`, or something else. Phase 2 must reference this artifact before implementing `nfs_mount`.

## Phase 1 - Make The NFS Server Path Compile On Windows

Purpose: make the direct AgentFS-to-NFS server usable from Windows without touching FUSE or sandboxing.

Tasks:

- In `cli/src/lib.rs`, change these exports from Unix-only to `#[cfg(any(unix, target_os = "windows"))]`:
  - `nfsserve`
  - `nfs`
  - `mount`
- In `cli/src/nfs.rs`, remove the CLI adapter's dependency on Unix-only libc APIs:
  - Add SDK-level open flag constants, for example `OPEN_READONLY = 0` and `OPEN_READWRITE = 2`, next to the `FileSystem::open` contract in `sdk/rust/src/filesystem/mod.rs`.
  - Re-export those constants from `agentfs-sdk` and use them in `cli/src/nfs.rs`.
  - Leave Unix-only HostFS internals free to translate those SDK constants to `libc` flags locally where needed.
  - Replace `major`, `minor`, and `makedev` calls with small target-independent helpers. For Windows/direct AgentFS the device value is usually `0`, but the helpers should still round-trip NFS special-file values.
- Keep `cli/src/nfsserve/fs_util.rs` gated out on Windows unless a later compile check proves it is needed.

Acceptance:

- `cargo check -p agentfs --target x86_64-pc-windows-msvc --no-default-features` reaches the command stubs or mount implementation work, not `nfsserve`/`nfs` compile errors.
- `cargo clippy -p agentfs --target x86_64-pc-windows-msvc --no-default-features --all-targets` is either clean or has documented pre-existing blockers.
- Linux/macOS checks still pass.

## Phase 2 - Add Windows Support To Generic Mount Infrastructure

Purpose: make `crate::mount::mount_fs` and RAII unmount work for Windows NFS.

Tasks:

- In `cli/src/mount/mod.rs`:
  - Add a Windows `mount_fs` implementation that allows only `MountBackend::Nfs`.
  - Keep `MountBackend::Fuse` as a clear Windows error.
  - Cfg-gate `MountHandle::drop` current-directory cleanup: leave the existing Unix `set_current_dir("/")` behavior unchanged, and add a Windows branch that moves out of the mounted drive before unmounting by using `std::env::temp_dir()` or another known non-mounted directory.
- In `cli/src/mount/nfs.rs`:
  - Add `#[cfg(target_os = "windows")] fn nfs_mount(port, mountpoint)`.
  - Add `#[cfg(target_os = "windows")] fn unmount_nfs(mountpoint, lazy)`.
  - Add parser helpers for `Z:` / `Z:\` mountpoints and reject directory paths with a clear message.
  - Implement the Windows NFS command from `work/windows-nfs-client-spike.md`; do not guess the non-default port syntax in code.
  - Add NFS client preflight helpers that find the Windows NFS client binaries and produce actionable setup guidance.
  - Add unit tests for drive-letter parsing and mount client detection logic where it can be tested without admin privileges.

Acceptance:

- `mount_fs(..., MountBackend::Nfs)` compiles on Windows.
- `mount_fs(..., MountBackend::Fuse)` bails on Windows with a Windows-specific message.
- Parser tests cover valid drive forms, assigned-looking directory paths, relative paths, and malformed drive strings.

## Phase 3 - Replace The Windows Mount Stub

Purpose: expose the working Windows NFS mount through `agentfs mount`.

Preferred implementation shape:

- Reuse `cli/src/cmd/mount.rs` for Windows instead of copying it to `mount_windows.rs`.
- Update `cli/src/cmd/mod.rs` to compile `mount.rs` for `#[cfg(any(unix, target_os = "windows"))]` and keep `mount_stub.rs` only for other unsupported targets.
- Add a Windows `pub fn mount(args: MountArgs)` branch in `cli/src/cmd/mount.rs`:
  - `MountBackend::Nfs` calls the existing NFS backend flow.
  - `MountBackend::Fuse` bails with a Windows-specific "use --backend nfs" message.

Windows-specific mount command work:

- Refactor mountpoint validation so Unix still canonicalizes existing directories, while Windows validates an unassigned drive letter and does not require `Z:` to exist before mounting.
- Make that refactor inside the shared `mount_nfs_backend` flow in `cli/src/cmd/mount.rs`, because the current `args.mountpoint.exists()` and `std::fs::canonicalize(...)` calls happen there before platform-specific NFS mounting.
- Initially support direct AgentFS databases first.
- If an AgentFS database has overlay base configuration before Windows `HostFS` lands, return a clear error such as "overlay-backed mounts require Windows HostFS support; direct AgentFS mounts are supported."
- Keep foreground/background behavior explicit: the NFS server process must remain alive for the drive mount to work. If true daemonization is not implemented, print that clearly.

Acceptance:

- `agentfs mount <id> Z: --backend nfs -f` reaches the Windows NFS client.
- `agentfs mount <id> Z: --backend fuse` fails before doing setup and explains the supported backend.
- Linux/macOS command behavior is unchanged.

## Phase 4 - Implement Windows HostFS

Purpose: unlock overlay-backed mounts and `agentfs run` on Windows.

Tasks:

- Add `sdk/rust/src/filesystem/hostfs_windows.rs`.
- Re-export `HostFS` on Windows from:
  - `sdk/rust/src/filesystem/mod.rs`
  - `sdk/rust/src/lib.rs`
- Implement the `FileSystem` trait using Windows APIs and `std::fs`:
  - Stable inode identity from Windows file identity metadata where available.
  - Preferred identity source: `GetFileInformationByHandleEx(FileIdInfo)` using `(VolumeSerialNumber, FILE_ID_128)`.
  - Fallback identity source: `GetFileInformationByHandle` using `(dwVolumeSerialNumber, nFileIndexHigh, nFileIndexLow)`.
  - Do not truncate the raw Windows identity directly into `Stats.ino`.
  - Map raw Windows identities to `Stats.ino: i64` through a deterministic 63-bit fingerprint over volume + file id, reserve `1` for the root, and maintain a per-`HostFS` bidirectional cache to detect collisions and assign a deterministic probe value for the process.
  - Ensure `lookup`, `getattr`, `readdir_plus`, `open`, and NFS `fileid3` conversion all use the same cache so repeated observations of the same file return the same inode while the `HostFS` instance is alive.
  - Document that the mapping is stable for normal identities and collision-safe within a process; if cross-process collision persistence is required later, add a persisted mount registry rather than changing `Stats.ino` semantics silently.
  - Path/inode cache comparable to Linux/macOS HostFS behavior.
  - `lookup`, `getattr`, `readdir`, `readdir_plus`, `open`, `mkdir`, `create_file`, `unlink`, `rmdir`, `rename`, `chmod`, `utimens`, `statfs`.
  - Conservative handling for `chown`, `mknod`, hard links, and symlinks. Prefer explicit unsupported errors over fake success.
  - POSIX-style mode synthesis from Windows metadata, including readonly bit handling.
- Add Windows-targeted unit tests for basic passthrough behavior and overlay copy-up behavior.

Acceptance:

- `agentfs-sdk` tests for direct filesystem operations pass on Windows.
- Overlay tests that do not rely on Unix-only permissions/special files pass on Windows or are explicitly cfg-gated with justification.
- Overlay-backed `agentfs mount ... --backend nfs` no longer needs the Phase 3 overlay error.

## Phase 5 - Implement `agentfs run` For Windows V1

Purpose: provide useful copy-on-write execution on Windows without overstating sandbox security.

Tasks:

- Replace `cli/src/cmd/run_windows.rs` with a real implementation.
- Use the macOS flow as the functional template:
  - Resolve/create run session.
  - Create delta AgentFS database.
  - Build `OverlayFS` over Windows `HostFS` for the current working directory.
  - Mount it through `crate::mount::mount_fs` using an unused drive letter.
  - Spawn the child with current directory set to the mounted drive root.
  - Drop the mount handle and clean up after child exit.
- Choose unused drive letters from high to low (`Z:` to `D:`) unless the user supplies one through a future option.
- Update `default_shell()` in `cli/src/main.rs` so Windows defaults to `cmd.exe` or PowerShell rather than `bash`.
- Add Windows shell prompt customization for the default interactive shell:
  - For `cmd.exe`, use a `/K` startup form that sets a visible AgentFS prompt.
  - For PowerShell, use a startup command that defines a `prompt` function.
  - For explicit non-shell commands, rely on environment variables rather than rewriting user arguments.
- Treat `--allow`, `--no-default-allows`, `--experimental-sandbox`, `--strace`, and `--system` carefully:
  - For v1, warn or error for sandbox-specific flags that cannot be enforced.
  - Set `AGENTFS=1`.
  - Set `AGENTFS_SANDBOX=windows-overlay-only`. This is an explicit "not a security sandbox" contract for downstream tools.
- Preserve session reuse for the delta database. Joining an already-mounted Windows session can be a later enhancement unless it falls out naturally.

Acceptance:

- `agentfs run cmd.exe /c "echo hello>created.txt"` writes into the AgentFS delta layer, not the original working tree.
- The command exit code is propagated.
- Cleanup unmounts the drive on normal exit and Ctrl+C where practical.
- Help text and startup banner make the no-sandbox limitation clear.

## Phase 6 - Secondary CLI Surfaces

Purpose: decide which adjacent commands should become Windows-supported after the core mount/run path is stable.

Tasks:

- `agentfs serve nfs` / legacy `agentfs nfs`:
  - Depends on Phase 1's `cli/src/nfs.rs` libc cleanup and `cli/src/lib.rs` export ungating.
  - Ungate `opts.rs`, `main.rs`, and `cmd/mod.rs` from Unix-only to `any(unix, windows)`.
  - Update printed client mount examples with Windows-specific guidance when running on Windows.
- `agentfs mount` list:
  - V1 decision: keep `agentfs_sdk::get_mounts()` Linux-only and keep Windows mount listing stubbed unless a CLI-owned registry is added.
  - If implementing Windows listing, first add a registry of agentfs-owned drive mounts. Parsing `mount.exe` output alone is not a safe source of ownership.
- `agentfs prune mounts`:
  - V1 decision: keep Windows prune stubbed until the registry above exists.
  - Once the registry exists, use `umount.exe` for known agentfs drive mounts only.
  - Never prune arbitrary NFS drives by guessing from `mount.exe` output.
- `agentfs init -c`:
  - Change `run_init_cmd` from Unix-only once `mount_fs` and Windows command execution are stable.
  - Use a temporary drive letter, not a temporary directory, for the mountpoint.
- `agentfs exec`:
  - Ungate only after mount/run behavior is proven.
  - Reuse the same temporary-drive helper as `init -c`.

Acceptance:

- Unsupported secondary surfaces fail explicitly.
- Supported secondary surfaces have Windows smoke tests.

## Phase 7 - Docs, Tests, And CI

Purpose: make the port maintainable and prevent regressions.

Tasks:

- Documentation:
  - Document Windows requirements: Client for NFS, edition/SKU limits, admin needs if any, drive-letter mountpoints, and no sandbox in v1.
  - Document FUSE/WinFsp as deferred, not silently unsupported.
- Unit tests:
  - Drive-letter parser.
  - Windows NFS client binary detection.
  - NFS device major/minor helpers.
  - Windows HostFS basic operations.
- CI:
  - Add `windows-latest` build/check/test jobs.
  - Keep integration tests gated on the NFS client feature being present.
  - Do not make CI depend on kernel-driver installs for v1.
- Manual smoke:
  - `agentfs init win-direct`
  - `agentfs fs win-direct write /hello.txt hello`
  - `agentfs mount win-direct Z: --backend nfs -f`
  - `Get-Content Z:\hello.txt`
  - create a file on `Z:\`, unmount, and verify through `agentfs fs`
  - `agentfs run cmd.exe /c "echo hello>created.txt"`
  - verify original CWD is unchanged and `agentfs diff <session>` shows the delta

Acceptance:

- `cargo fmt --all`
- `cargo clippy -p agentfs --target x86_64-pc-windows-msvc --all-targets`
- `cargo test -p agentfs-sdk --target x86_64-pc-windows-msvc`
- `cargo test -p agentfs --target x86_64-pc-windows-msvc`
- Linux/macOS checks still pass for changed crates.

## V2 Backlog

- AppContainer or Job Object based confinement for `agentfs run`.
- WinFsp native backend or WinFsp-NFS fallback for Windows Home and better drive integration.
- Directory mount exposure through junctions or `subst`-style mapping if drive letters are not acceptable.
- Background Windows service/process model for long-lived mounts.
- Robust mount registry for list/prune/session-join behavior.
- Deeper symlink, case-sensitivity, ACL, and special-file compatibility work.

## Main Risks

- Windows Client for NFS availability differs by edition and optional feature state.
- The exact non-default-port UNC syntax must be verified before coding.
- Windows drive-letter mounts change CLI semantics compared with Unix directory mountpoints.
- Windows `HostFS` is real work; `run` depends on it.
- Without AppContainer/Job Object confinement, `run` is not a security boundary.
- NFS semantics around case sensitivity, locking, UID/GID, symlinks, and special files may differ from Unix clients.

## Recommended Commit Sequence

1. Windows compile gates and NFS adapter cleanup.
2. Windows NFS mount/unmount helpers plus parser tests.
3. `agentfs mount --backend nfs` direct-database support.
4. Windows `HostFS` and overlay mount support.
5. Windows `run` without sandbox.
6. Secondary CLI surfaces, docs, and CI.
