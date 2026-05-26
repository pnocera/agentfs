# Phase 3 Windows Mount Command

## Scope

Phase 3 exposed the Windows NFS backend through `agentfs mount` for direct
AgentFS databases.

Changed files:

- `cli/src/cmd/mod.rs`
- `cli/src/cmd/mount.rs`
- `cli/src/mount/mod.rs`
- `cli/src/mount/nfs.rs`

## Implementation Notes

- `cli/src/cmd/mod.rs` now compiles the real mount command on Windows via
  `#[cfg(any(unix, target_os = "windows"))]`.
- Added a Windows `pub fn mount(args: MountArgs)` branch:
  - `--backend fuse` fails before setup with a Windows-specific "use
    --backend nfs" message.
  - `--backend nfs` requires foreground mode so the in-process NFS server stays
    alive for the drive mount.
  - Foreground NFS calls the shared `mount_nfs_backend` flow.
- Refactored mountpoint handling in the shared NFS backend:
  - Unix still requires an existing mountpoint and canonicalizes it.
  - Windows keeps the raw drive letter, such as `Z:`, and leaves validation to
    the Windows NFS mount helper.
- Direct AgentFS databases are supported on Windows.
- Overlay-backed mount requests on Windows now fail clearly before constructing
  a `HostFS`: `overlay-backed mounts require Windows HostFS support; direct
  AgentFS mounts are supported`.
- Background Windows NFS mounts fail clearly because no Windows daemon/service
  model exists yet.
- Windows `agentfs mount` list and prune surfaces remain stubs with explicit
  Windows messages.
- Windows NFS now defaults to port `2049`. This machine can bind that port as a
  normal user, and the built-in Windows Client for NFS did not accept the
  previously tested high-port localhost forms.
- When using port `2049`, the Windows NFS helper probes these source forms in
  order:
  - `\\127.0.0.1\!`
  - `\\127.0.0.1@2049\!`
  - `\\127.0.0.1:2049\!`
- For non-default ports, the helper still probes:
  - `\\127.0.0.1@PORT\!`
  - `\\127.0.0.1:PORT\!`

## Live Windows NFS Client Result

The Phase 3 smoke command reaches the Windows Client for NFS:

```powershell
F:\Tools\agentfs\cli\target\x86_64-pc-windows-msvc\debug\agentfs.exe mount :memory: Z: --backend nfs -f
```

On this machine the client rejected all supported localhost source forms with
Network Error 53:

- `\\127.0.0.1\!`
- `\\127.0.0.1@2049\!`
- `\\127.0.0.1:2049\!`

This satisfies the Phase 3 command-path acceptance criterion of reaching the
Windows NFS client, but it is not a successful live drive mount. Later phases
must either add the missing Windows-compatible NFS service surface, adopt a
WinFsp-backed mount path, or narrow the Windows NFS client support contract.

## Verification

Commands were run from `F:\Tools\agentfs\cli` after loading:

```cmd
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat
```

with `C:\Program Files\LLVM\bin` prepended to `PATH`.

| Command | Result | Log |
|---|---:|---|
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase3-cargo-check-windows-no-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --no-default-features --all-targets` | PASS | `work/phase3-cargo-clippy-windows-no-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --no-default-features mount::` | PASS | `work/phase3-cargo-test-windows-mount.log` |
| `cargo check --target x86_64-pc-windows-msvc` | PASS | `work/phase3-cargo-check-windows-default-features.log` |
| `cargo build --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase3-cargo-build-windows-no-default-features.log` |
| `agentfs.exe mount :memory: Z: --backend fuse` | PASS, expected failure | `work/phase3-cli-smoke-windows-mount-fuse.log` |
| `agentfs.exe mount :memory: Z: --backend nfs -f` | REACHES CLIENT, mount fails with Error 53 | `work/phase3-cli-smoke-windows-mount-nfs.log` |

The refreshed Windows mount tests cover:

- Windows `agentfs mount --backend fuse` rejection before setup.
- Windows background NFS rejection.
- Windows `mount_fs(..., MountBackend::Fuse)` rejection.
- Windows drive-letter parsing.
- System32 NFS client path construction.
- Non-default-port source construction.
- Default-port source construction with the no-port form first.

Clippy still reports the same three pre-existing warnings outside the Phase 3
files:

- `src/cmd/init.rs:109` redundant reference in `format!`.
- `src/cmd/ps.rs:219` redundant reference in `writeln!`.
- `src/cmd/run_not_supported.rs:9` `too_many_arguments`.

Linux and macOS compile checks from this phase are saved at:

- `work/phase3-cargo-check-linux-cli-no-default-features.log`
- `work/phase3-cargo-test-linux-cli-mount.log`
- `work/phase3-cargo-check-macos-cli-no-default-features.log`
