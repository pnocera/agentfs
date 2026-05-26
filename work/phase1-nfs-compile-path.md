# Phase 1 NFS Compile Path

## Scope

Phase 1 widened the Rust CLI NFS modules so they compile on Windows and removed
the NFS adapter's dependency on Unix-only libc open/device helpers.

Changed files:

- `sdk/rust/src/filesystem/mod.rs`
- `sdk/rust/src/filesystem/hostfs_darwin.rs`
- `sdk/rust/src/lib.rs`
- `cli/src/lib.rs`
- `cli/src/nfs.rs`
- `cli/src/mount/mod.rs`
- `cli/src/mount/nfs.rs`

## Implementation Notes

- Added SDK-level open access constants:
  - `OPEN_READONLY = 0`
  - `OPEN_WRITEONLY = 1`
  - `OPEN_READWRITE = 2`
- Re-exported the constants from `agentfs-sdk`.
- Updated `FileSystem::open` documentation to reference SDK constants rather
  than `libc::O_RDONLY` / `libc::O_RDWR`.
- Changed `nfsserve`, `nfs`, and `mount` module gates from Unix-only to
  `any(unix, target_os = "windows")`.
- Replaced the NFS adapter's `libc::O_RDONLY` / `libc::O_RDWR` usage with the
  SDK open constants.
- Added SDK-level `device_major`, `device_minor`, and `make_device` helpers
  using a stable Linux/GNU-compatible device ID encoding.
- Documented `Stats.rdev` as SDK-encoded via `make_device`.
- Updated the NFS adapter to use the SDK device helpers instead of
  `libc::major`, `libc::minor`, and `libc::makedev`.
- Updated macOS HostFS to translate between Darwin `dev_t` packing and the
  SDK device encoding at the HostFS boundary.
- Added focused CLI and SDK unit tests for major/minor round trips.
- Added Windows NFS mount/unmount stubs in the mount backend. These keep Phase
  1 compiling while leaving real Windows NFS client detection and mount command
  invocation to Phase 2.

## Verification

### Windows

Windows commands were run from `F:\Tools\agentfs\cli` or
`F:\Tools\agentfs\sdk\rust` after loading:

```cmd
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat
```

with `C:\Program Files\LLVM\bin` prepended to `PATH`.

| Command | Result | Log |
|---|---:|---|
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase1-cargo-check-windows-no-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc --no-default-features --all-targets` | PASS | `work/phase1-cargo-clippy-windows-no-default-features.log` |
| `cargo check --target x86_64-pc-windows-msvc` | PASS | `work/phase1-cargo-check-windows-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --no-default-features device_encoding_round_trips_major_minor` in `cli` | PASS | `work/phase1-cargo-test-windows-cli-device-encoding.log` |
| `cargo test --target x86_64-pc-windows-msvc device_encoding_round_trips_major_minor` in `sdk/rust` | PASS | `work/phase1-cargo-test-windows-sdk-device-encoding.log` |

Clippy still reports three pre-existing warnings outside the Phase 1 files:

- `src/cmd/init.rs:109` redundant reference in `format!`.
- `src/cmd/ps.rs:219` redundant reference in `writeln!`.
- `src/cmd/run_not_supported.rs:9` `too_many_arguments`.

### Linux

Linux commands were run in Ubuntu WSL from `/mnt/f/Tools/agentfs`. The WSL
environment needed `liblzma-dev` installed before tests could link because
`cargo test` initially failed at `rust-lld: unable to find library -llzma`.

| Command | Result | Log |
|---|---:|---|
| `cargo check --target x86_64-unknown-linux-gnu --no-default-features` in `cli` | PASS | `work/phase1-cargo-check-linux-cli-no-default-features.log` |
| `cargo test --target x86_64-unknown-linux-gnu --no-default-features` in `cli` | PASS | `work/phase1-cargo-test-linux-cli-no-default-features.log` |
| `cargo check --target x86_64-unknown-linux-gnu` in `sdk/rust` | PASS | `work/phase1-cargo-check-linux-sdk.log` |
| `cargo test --target x86_64-unknown-linux-gnu` in `sdk/rust` | PASS | `work/phase1-cargo-test-linux-sdk.log` |

### macOS

macOS checks were cross-target `cargo check` runs from Windows, not native macOS
test execution. The repo selects nightly in this directory, so the
`x86_64-apple-darwin` Rust target was installed for
`nightly-x86_64-pc-windows-msvc`. The C portions were checked with the local Zig
0.16 toolchain:

```cmd
PATH=C:\Users\Pierre\AppData\Roaming\Code\User\globalStorage\ziglang.vscode-zig\zig\x86_64-windows-0.16.0-dev.1301+cbfa87cbe;C:\Program Files\LLVM\bin;%PATH%
CC_x86_64_apple_darwin=zig cc -target x86_64-macos
AR_x86_64_apple_darwin=zig ar
CRATE_CC_NO_DEFAULTS=1
CFLAGS_x86_64_apple_darwin=-isystem C:\Users\Pierre\AppData\Roaming\Code\User\globalStorage\ziglang.vscode-zig\zig\x86_64-windows-0.16.0-dev.1301+cbfa87cbe\lib\libc\include\any-macos-any
```

| Command | Result | Log |
|---|---:|---|
| `cargo check --target x86_64-apple-darwin --no-default-features` in `cli` | PASS | `work/phase1-cargo-check-macos-cli-no-default-features.log` |
| `cargo check --target x86_64-apple-darwin` in `sdk/rust` | PASS | `work/phase1-cargo-check-macos-sdk.log` |

## Phase 2 Handoff

Windows now compiles through the NFS adapter and into mount backend stubs. Phase
2 should replace those stubs with Windows NFS client detection and `mount.exe` /
`umount.exe` command invocation, including live validation of the Windows NFS
UNC form against a running AgentFS NFS server.

Before Phase 2 introduces a Windows `mount_fs`, replace
`MountHandle::drop`'s unconditional `set_current_dir("/")` with a Windows branch
that moves to `std::env::temp_dir()` or another path outside the mounted drive.

When Phase 2 wires Windows `mount_nfs`, order the flow as preflight client
detection, bind server, invoke `mount.exe`, and cancel/abort the spawned NFS
task if the mount command fails. The current Linux/macOS shape assumes the
mount helper failure path is short-lived and relies on `Drop` after a
`MountHandle` exists.
