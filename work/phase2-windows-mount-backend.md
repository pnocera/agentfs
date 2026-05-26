# Phase 2 Windows Mount Backend

## Scope

Phase 2 made the generic mount infrastructure compile and dispatch on Windows
for the NFS backend only.

Changed files:

- `cli/src/mount/mod.rs`
- `cli/src/mount/nfs.rs`

## Implementation Notes

- Added a Windows `mount_fs` implementation.
  - `MountBackend::Nfs` calls the NFS backend.
  - `MountBackend::Fuse` bails with a Windows-specific message.
- Split `MountHandle::drop` current-directory cleanup:
  - Unix keeps `set_current_dir("/")`.
  - Windows moves to `std::env::temp_dir()` before unmounting.
- Kept the NFS server task alive during unmount, then aborts it after the
  unmount attempt. If the mount command fails before a `MountHandle` exists,
  the task is aborted immediately.
- Added Windows Client for NFS preflight:
  - Uses `%SystemRoot%\System32\mount.exe`.
  - Uses `%SystemRoot%\System32\umount.exe`.
  - Checks that `NfsClnt` exists via `%SystemRoot%\System32\sc.exe query NfsClnt`.
  - Returns actionable guidance for enabling `ServicesForNFS-ClientOnly` and
    `ClientForNFS-Infrastructure`.
- Added drive-letter validation for Windows mountpoints.
  - Accepts `Z:`, `Z:\`, and `Z:/`.
  - Rejects directories, relative paths, malformed drive strings, and non-letter
    drives.
- Implemented the Windows NFS mount probe from `work/windows-nfs-client-spike.md`.
  The backend tries these non-default-port source forms against the live server:
  - `\\127.0.0.1@PORT\!`
  - `\\127.0.0.1:PORT\!`
- Uses the Phase 0 recommended Windows Client for NFS options:
  `anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1`.
- Windows unmount uses `umount.exe`, adding `-f` when the handle was configured
  for lazy unmount.

## Verification

Commands were run from `F:\Tools\agentfs\cli` after loading:

```cmd
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat
```

with `C:\Program Files\LLVM\bin` prepended to `PATH`.

| Command | Result | Log |
|---|---:|---|
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase2-cargo-check-windows-no-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --no-default-features --all-targets` | PASS | `work/phase2-cargo-clippy-windows-no-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --no-default-features mount::` | PASS | `work/phase2-cargo-test-windows-mount.log` |
| `cargo check --target x86_64-pc-windows-msvc` | PASS | `work/phase2-cargo-check-windows-default-features.log` |

The Windows mount tests cover:

- Valid drive forms.
- Directory-like paths.
- Relative paths.
- Malformed drive strings.
- System32 NFS client path construction.
- Non-default-port probe source construction.
- `mount_fs(..., MountBackend::Fuse)` Windows rejection.

Clippy still reports the same three pre-existing warnings outside Phase 2 files:

- `src/cmd/init.rs:109` redundant reference in `format!`.
- `src/cmd/ps.rs:219` redundant reference in `writeln!`.
- `src/cmd/run_not_supported.rs:9` `too_many_arguments`.

No live Windows NFS mount smoke was run in Phase 2. The implementation now
performs the live source-form probe at runtime once Phase 3 exposes
`agentfs mount` on Windows.
