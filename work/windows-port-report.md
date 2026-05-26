# agentfs Windows port — scoping report

**Date:** 2026-05-26
**Repo:** `F:\Tools\agentfs` (fork of tursodatabase/agentfs, v0.6.4)
**Audience:** you, the person who's going to write the patches.

---

## TL;DR

- The two interesting subcommands on Windows are `mount` and `run`. Both currently `bail!` upstream of any backend decision — WinFsp being installed changes nothing until upstream code calls into it.
- The **FUSE backend cannot be reused as-is** on Windows. agentfs vendors a *pure-Rust* `fuser` (`cli/src/fuser/**`) that opens `/dev/fuse` and issues `mount(2)` via `libc`/`nix`. WinFsp's libfuse-compatible shim is a C-API shim; the vendored fuser doesn't call libfuse at all. So "swap in WinFsp" is not a small change.
- The **NFS backend is already mostly cross-platform**. The vendored NFS server (`cli/src/nfsserve/**`) only has one Unix-only module (`fs_util.rs`, `cfg(not(windows))` gated at `cli/src/nfsserve/mod.rs:19`). Everything else is pure async-Rust over TCP. This is the shortest path.
- The **macOS implementation is the right template for Windows**: it skips FUSE entirely, runs the NFS server on localhost, mounts via the OS NFS client, and confines the child with the OS's native sandbox primitive (`sandbox-exec`). On Windows the equivalent triplet is: same NFS server → Windows built-in NFS client (or WinFsp-NFS) → Job Object / AppContainer.
- **Recommended path**: ship `mount --backend nfs` on Windows first (small patch, real value). Then `run` using the macOS template adapted to Windows. WinFsp only becomes interesting later as a third option for `mount` if NFS perf/semantics aren't good enough.

---

## 1. Current architecture & where Windows is blocked

### 1.1 Platform-gated module routing

`cli/src/cmd/mod.rs:10-14` decides at compile time which `mount` implementation gets compiled in:

```rust
#[cfg(unix)]
pub mod mount;
#[cfg(not(unix))]
#[path = "mount_stub.rs"]
pub mod mount;
```

`cli/src/cmd/run.rs:10-17` does the same for `run`:

```rust
#[cfg_attr(all(target_os = "linux", feature = "sandbox"), path = "run_linux.rs")]
#[cfg_attr(all(target_os = "macos", feature = "sandbox"), path = "run_darwin.rs")]
#[cfg_attr(
    all(target_os = "windows", feature = "sandbox"),
    path = "run_windows.rs"
)]
#[cfg_attr(not(feature = "sandbox"), path = "run_not_supported.rs")]
mod sys;
```

Note: `run_windows.rs` and `run_not_supported.rs` are *both* present, both bail. The Windows file has been scaffolded but never implemented. `cli/src/cmd/init.rs:271` also bails out for `init -c` on Windows ("The -c option is not supported on Windows").

### 1.2 The CLI surface is **already exposed** on Windows

Important: `Mount` (`cli/src/opts.rs:217`) and `Run` (`cli/src/opts.rs:138`) are **not** `#[cfg(unix)]`-gated in `opts.rs`. So the subcommands show up in `agentfs --help` on Windows — it's only the implementation that bails. Means: adding a real Windows impl is a drop-in; no CLI plumbing changes needed.

Things that *are* `#[cfg(unix)]` gated in `opts.rs` and would also need ungating:
- `Exec` subcommand (`cli/src/opts.rs:190`)
- Legacy `Nfs` subcommand (`cli/src/opts.rs:285`)
- `Serve::Nfs` subcommand (`cli/src/opts.rs:376`)

### 1.3 Backend abstraction

There is no `Backend` trait. Dispatch is monolithic function branching on `MountBackend` (`cli/src/opts.rs:9-16`):

```rust
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MountBackend {
    /// FUSE filesystem (Linux only)
    Fuse,
    /// NFS over localhost
    Nfs,
}
```

Per-OS dispatch in `cli/src/cmd/mount.rs:58-83`:
- Linux: `Fuse → mount_fuse`, `Nfs → mount_nfs_backend` via tokio runtime
- macOS: `Fuse → bail("use --backend nfs")`, `Nfs → mount_nfs_backend`
- Windows: doesn't reach here — the whole module is replaced by `mount_stub.rs`

This is good news for porting: it means we don't need to refactor to a trait first. Just add a `#[cfg(target_os = "windows")] pub fn mount(...)` parallel to the existing ones.

### 1.4 Dependency gates in `cli/Cargo.toml`

```toml
# Unix dependencies
[target.'cfg(unix)'.dependencies]
libc = "0.2"

# Linux-only dependencies for FUSE and sandbox functionality
[target.'cfg(target_os = "linux")'.dependencies]
agentfs-sandbox = { path = "../sandbox", optional = true }
reverie = { ... optional = true }
# Vendored fuser dependencies (pure-Rust FUSE implementation)
log = "0.4"
memchr = "2.7"
page_size = "0.6"
smallvec = "1.6"
zerocopy = { version = "0.8", features = ["derive"] }
nix = { version = "0.29", features = ["fs", "user"] }

# macOS dependencies for NFS functionality (no FUSE - uses nfsserve instead)
[target.'cfg(target_os = "macos")'.dependencies]
aegis = { version = "0.9.6", features = ["pure-rust"] }
```

So the vendored `fuser` deps (memchr, smallvec, zerocopy, nix, …) are themselves Linux-only at the Cargo level. The `fuser` code in `cli/src/fuser/**` is presumably gated module-side, but inherits those deps. Nothing on the Windows side today.

### 1.5 Feature flags

```toml
[features]
default = ["sandbox"]
sandbox = [ "dep:agentfs-sandbox", "dep:reverie", ... ]
```

The `sandbox` feature is in the default set but its deps are all `[target.'cfg(target_os = "linux")']`-gated. On Windows, the feature is "on" but contributes nothing — and `run.rs`'s `#[cfg_attr(all(target_os = "windows", feature = "sandbox"), path = "run_windows.rs")]` still routes to the bail stub.

---

## 2. The FUSE constraint — why WinFsp isn't a drop-in

`cli/Cargo.toml:64` calls it out: *"Vendored fuser dependencies (pure-Rust FUSE implementation)"*. The actual code lives in `cli/src/fuser/**`:

```
cli/src/fuser/channel.rs
cli/src/fuser/mnt/fuse_pure.rs    ← opens /dev/fuse, mount(2)
cli/src/fuser/mnt/mod.rs
cli/src/fuser/mnt/mount_options.rs
cli/src/fuser/session.rs
cli/src/fuser/ll/fuse_abi.rs       ← FUSE wire protocol structs
cli/src/fuser/ll/request.rs
cli/src/fuser/ll/reply.rs
cli/src/fuser/notify.rs
cli/src/fuser/reply.rs
cli/src/fuser/request.rs
cli/src/fuser/mod.rs
```

This is a from-scratch implementation of the FUSE *kernel protocol*, not a wrapper around libfuse. It reads/writes FUSE messages on `/dev/fuse` directly. WinFsp ships a libfuse-API-compatible shim (`winfsp-x64.dll`) — but agentfs doesn't link libfuse, so the shim is invisible to it.

### What this means for a "FUSE on Windows" effort

Three sub-options, in order of effort:

1. **Rewrite the mount layer + I/O loop to target WinFsp's native C API.** Use `winfsp-sys` (FFI bindings, exists on crates.io) or the higher-level `winfsp` crate. Map FUSE operations → WinFsp callbacks. This is a few-thousand-line endeavor: WinFsp has its own model (`FSP_FILE_SYSTEM_INTERFACE` with `Create/Open/Read/Write/...`), and you'd be writing an adapter from the existing AgentFS `FileSystem` trait to that. **High effort, high payoff** (real drive-letter mount, transparent to all Win32 apps).

2. **Add libfuse as an external dep, drop the vendored fuser on Windows, link against WinFsp's shim.** This means replacing `cli/src/fuser/**` with a wrapper around the `fuse3-sys` or `libfuse-sys` crate — gated by OS. Lower up-front code but adds a real C dep on all platforms (or platform-conditional crate selection), and the WinFsp libfuse shim has known incompatibilities vs. real libfuse around symlinks/xattrs. **Medium effort, fragile.**

3. **Skip FUSE on Windows entirely.** Make `--backend fuse` an error on Windows like it is on macOS. **Zero effort, ships today, perfectly defensible.** This is what macOS does (`cli/src/cmd/mount.rs:72-77`).

Option 3 is the recommendation for v1. The story becomes: *"On Windows, agentfs uses NFS just like macOS."*

---

## 3. The NFS backend — almost free

`cli/src/nfsserve/**` is a vendored, custom NFSv3 server. Module list at `cli/src/nfsserve/mod.rs:1-24`:

```rust
mod context;
pub mod permissions;
pub mod rpc;
mod rpcwire;
mod write_counter;
pub mod xdr;

mod mount;
mod mount_handlers;
mod portmap;
mod portmap_handlers;

pub mod nfs;
mod nfs_handlers;

#[cfg(not(target_os = "windows"))]
pub mod fs_util;

pub mod tcp;
mod transaction_tracker;
pub mod vfs;
```

**Only one module is Windows-gated**: `fs_util.rs`. Everything else is std/tokio/async-trait. The gate at line 19 was put there because `fs_util.rs` uses `std::os::unix::fs::{MetadataExt, PermissionsExt}` — `MetadataExt::ino()` and friends.

### What `fs_util.rs` does

The subagent's read indicated it's used for `metadata_differ()` (comparing two host metadata snapshots for change detection — inode, mtime, etc.). If the NFS-server side of agentfs (the **server** that lives inside agentfs and exposes the AgentFS filesystem) needs `fs_util` for its core ops, this is a blocker. If it's only used in `HostFS`-style passthrough cases, it might already not run when serving an AgentFS overlay.

**Action item for the porter:** grep `fs_util::` across `cli/src/` to see if it's actually exercised by `mount_nfs_backend`. If not used → trivial; the existing `#[cfg(not(windows))]` gate is already enough and the server compiles on Windows untouched.

### The mount-client side — separate problem

The NFS *server* compiling on Windows is necessary but not sufficient. `mount --backend nfs` also needs to *mount* the resulting NFS export. `cli/src/cmd/mount.rs:332-382` (per subagent) shells out to:

- Linux: `mount -t nfs -o ... 127.0.0.1:/ <mp>`
- macOS: `/sbin/mount_nfs -o ... 127.0.0.1:/ <mp>`

For Windows, the two viable options:

1. **Windows built-in NFS client** (`mount.exe` from "Services for NFS"). Available on Windows 10/11 Pro & Enterprise as an optional Windows feature ("Client for NFS"). Not in Home edition. Invocation: `mount.exe -o anon \\127.0.0.1\! Z:`. Mounts to a drive letter, not a directory path.
2. **WinFsp's NFS bridge**, if you don't want to require the optional Windows feature.

Recommendation: try the built-in client first. Detect at runtime with `where.exe mount.exe` (the Windows `mount.exe` from Services for NFS is distinct from the unrelated UNIX-emulation `mount.exe` from things like Git Bash — check via `mount.exe /?` output for the SFU banner).

### The drive-letter vs. directory mismatch

Windows NFS client only mounts to **drive letters**, not arbitrary directory paths. The current `MountArgs.mountpoint: PathBuf` semantic on Unix is a directory. For Windows, options:

- Accept a drive letter as mountpoint and validate (e.g., `Z:` or `Z:\`).
- Accept a directory and surface a clear error: "On Windows, the mountpoint must be an unassigned drive letter (e.g. `Z:`)."
- Auto-pick an unused letter if the user passes `auto` or similar.

---

## 4. The `run` subcommand — model after macOS

Per the subagent's read of `cli/src/cmd/run.rs`, `run_linux.rs`, `run_darwin.rs`:

- **Linux `run`**: FUSE-mounts the AgentFS overlay onto a hidden dir, then uses `mount(2)` + `unshare(2)` to bind-mount it onto the child's cwd inside a new mount namespace. Child sees the overlay as its real filesystem. (Or: experimental ptrace mode via reverie.)
- **macOS `run`**: No FUSE, no namespaces. Spins up the NFS server on `127.0.0.1`, mounts via `/sbin/mount_nfs` to a session-specific directory, then `sandbox-exec -p <profile> <cmd>` confines the child to that directory + allowed paths. Sandbox profile is generated per-run.

The Windows analogue would be:

1. Same NFS server (assuming §3 is done).
2. `mount.exe` via Services for NFS (or WinFsp), to a session-allocated drive letter.
3. Confine the child with one of:
   - **Job Object** with UI restrictions + filesystem ACLs that deny access to non-allowed host paths. Crude but native.
   - **AppContainer** — Microsoft's userland sandbox primitive. Per-app, capability-based, harder to set up than a Job Object. Closer in spirit to sandbox-exec.
   - **Windows Sandbox** — full lightweight VM. Heavy, slow start, but maximum isolation.
   - **Nothing** — first cut could just `cd <mountpoint> && <cmd>` and skip sandboxing entirely. Document the limitation.

Recommend: start with **no sandbox** (option d) for v1, plus a `--no-sandbox` flag that documents the limitation explicitly. Add AppContainer in a second pass. This gets `run` working end-to-end on Windows without needing to learn Job Object internals first.

---

## 5. Concrete file-by-file work plan (recommended path)

Minimum-viable Windows port = `mount --backend nfs` working. Then `run` using mount under the hood. WinFsp left for v2.

### Step 0 — verify NFS server compiles on Windows

```powershell
cd F:\Tools\agentfs
cargo build --target x86_64-pc-windows-msvc -p agentfs --no-default-features 2>&1 | Tee-Object windows-build-step0.log
```

Expected: lots of errors from `cli/src/cmd/mount_stub.rs` and `run_windows.rs` (fine — they're shims) but the `nfsserve` module should compile. Triage what doesn't.

### Step 1 — un-gate the NFS-related CLI surfaces on Windows

In `cli/src/opts.rs`, change these from `#[cfg(unix)]` to something more permissive — either remove the gate, or change to `#[cfg(any(unix, target_os = "windows"))]` (i.e. everything-but-`wasi`/`fuchsia`):

- line ~285: `Nfs { ... }` (legacy)
- line ~376: `Serve::Nfs { ... }`
- line ~190: `Exec { ... }` (only if §6 lands)

`Mount` and `Run` are already ungated.

### Step 2 — replace `mount_stub.rs` with a real Windows impl

Rename `cli/src/cmd/mount_stub.rs` → `cli/src/cmd/mount_windows.rs`. Update `cli/src/cmd/mod.rs:10-14`:

```rust
#[cfg(unix)]
pub mod mount;
#[cfg(windows)]
#[path = "mount_windows.rs"]
pub mod mount;
```

Implement two paths in `mount_windows.rs`:

```rust
pub fn mount(args: MountArgs) -> Result<()> {
    match args.backend {
        MountBackend::Fuse => bail!(
            "FUSE mounting is not supported on Windows.\n\
             Use --backend nfs (default) instead.\n\
             (WinFsp support is on the roadmap — see work/windows-port-report.md)"
        ),
        MountBackend::Nfs => {
            let rt = crate::get_runtime();
            rt.block_on(mount_nfs_backend_windows(args))
        }
    }
}
```

`mount_nfs_backend_windows` mirrors `mount_nfs_backend` in `cli/src/cmd/mount.rs` but uses the Windows mount client. Bulk of it can probably move into a shared `mount_nfs_backend` and only the final "mount via OS client" step is platform-specific.

### Step 3 — Windows NFS client invocation

New function, somewhere near the existing `nfs_mount`:

```rust
#[cfg(windows)]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let drive = parse_drive_letter(mountpoint)?; // e.g. "Z:"
    let unc = format!(r"\\127.0.0.1@{}\!", port);
    let status = Command::new("mount.exe")
        .args(["-o", "anon,nolock,casesensitive=yes", &unc, drive])
        .status()
        .context("running mount.exe — is Windows 'Services for NFS / Client for NFS' enabled?")?;
    if !status.success() {
        bail!("mount.exe failed with exit code {:?}", status.code());
    }
    Ok(())
}
```

Notes:
- The `@{port}` in the UNC path is how Windows NFS client passes a non-default port. Verify against Microsoft docs for the exact form (`\\server:port\share` vs `\\server@port\share`).
- Add a preflight check: error early with a clear message if `mount.exe` isn't available *or* it's the wrong `mount.exe` (Git Bash ships one too).

### Step 4 — `prune mounts` and `list mounts`

`cli/src/cmd/mount_stub.rs:31-43` has stubs for these. Replace:

- `list_mounts`: query Windows for current NFS mounts (`mount.exe` with no args lists them on SFU).
- `prune_mounts`: `umount.exe <drive>:` for each agentfs-tagged mount.

Identifying "agentfs-tagged" mounts: same approach as Unix (compare against the registry of active sessions in the agentfs DB — `cli/src/cmd/ps.rs` knows this).

### Step 5 — `run` for Windows v1 (no sandbox)

Replace `cli/src/cmd/run_windows.rs` bail with:

```rust
pub async fn run(
    allow: Vec<PathBuf>,
    no_default_allows: bool,
    _experimental_sandbox: bool, // ignored
    _strace: bool,                // ignored
    session_id: Option<String>,
    _system: bool,
    encryption: Option<(String, String)>,
    command: PathBuf,
    args: Vec<String>,
) -> Result<()> {
    // 1. Same setup as run_darwin.rs:
    //    - resolve session, open AgentFS + OverlayFS, init base
    // 2. Spin up NFS server on localhost
    // 3. Pick an unused drive letter (Z:, Y:, X:, ...)
    // 4. mount.exe to that letter
    // 5. Spawn `command` with cwd = drive letter, no sandbox
    // 6. Wait, capture exit code, unmount, abort server
}
```

Cribbing 80% from `run_darwin.rs` is the right move — the only Windows-specific bits are mount/unmount and the (absent) sandbox step.

### Step 6 — handle `fs_util` if it's needed

If Step 0 surfaces compile errors from someone *using* `fs_util::` on Windows, port the two metadata calls:

- `MetadataExt::ino()` → `GetFileInformationByHandle` → `FILE_INFORMATION_BY_HANDLE.nFileIndex{High,Low}` (the Windows file index)
- `MetadataExt::mtime()` → `Metadata::modified()` (stable, cross-platform)
- `PermissionsExt::mode()` → use a stable u32 that just encodes readonly bit (Windows doesn't have POSIX modes)

Wrap in a small `cfg`-divergent helper.

### Step 7 — CI

Add a `windows-latest` job to `.github/workflows/`:
```yaml
- runs-on: windows-latest
- name: enable NFS client
  run: Enable-WindowsOptionalFeature -Online -FeatureName ServicesForNFS-ClientOnly,ClientForNFS-Infrastructure -NoRestart
- run: cargo build
- run: cargo test --workspace
```

End-to-end smoke test for `mount --backend nfs` and `run` requires the NFS client feature; gate that test on the feature being present.

### Step 8 (optional, v2) — WinFsp backend

Add `winfsp-sys` (or higher-level `winfsp` crate) as a Windows-only dep. Implement `mount_winfsp(args)` parallel to `mount_fuse`. Wire it into the dispatcher in `mount_windows.rs`. The bulk of the work is implementing the WinFsp `FSP_FILE_SYSTEM_INTERFACE` callbacks as an adapter over `agentfs_sdk::FileSystem`. Plan on this being its own multi-week project.

---

## 6. Open questions / decisions you'll want to make first

1. **Edition / SKU support**: do you require Windows 10/11 Pro+ (built-in NFS client available), or do you want Home edition to work too? If Home: WinFsp becomes mandatory, not optional. → Decision before Step 3.

2. **Drive letter vs. junction**: NFS client mounts to a drive letter. Are you OK with that being agentfs's contract on Windows, or do you want to layer junction points / `subst` to expose a directory path? Adds complexity.

3. **MSI vs. portable**: WinFsp install is MSI-only with a kernel driver (needs admin). Are you OK with the documentation saying "install WinFsp separately"? Most projects in this space (rclone, SSHFS-Win) do this. → Affects how you write the README.

4. **Test infrastructure**: GitHub Actions `windows-latest` doesn't let you install kernel drivers easily. WinFsp install on CI is known to work via the official MSI + `msiexec /quiet`. NFS client feature install on CI also works. → Both testable; matters for which gets CI coverage first.

5. **`run` sandbox seriousness**: is "no sandbox, just an overlay" acceptable for v1 Windows? Or is the sandbox the whole point and you want AppContainer from day 1? → Decision before Step 5.

6. **Upstream contribution**: are you doing this as a fork-only port for your own use, or upstreaming to tursodatabase/agentfs? The architecture decisions (especially the WinFsp dep posture) should align with upstream maintainers' preferences if upstreaming. Worth opening a "Windows support — design discussion" issue before significant code.

---

## 7. References (in-repo)

| Concern | Path | Lines |
| --- | --- | --- |
| Module routing for `mount` | `cli/src/cmd/mod.rs` | 10–14 |
| Module routing for `run` | `cli/src/cmd/run.rs` | 10–17 |
| Mount Windows stub | `cli/src/cmd/mount_stub.rs` | 1–43 |
| Run Windows stub | `cli/src/cmd/run_windows.rs` | 1–22 |
| Run "not supported" stub | `cli/src/cmd/run_not_supported.rs` | 1–21 |
| `init -c` Windows bail | `cli/src/cmd/init.rs` | 271 |
| Backend enum | `cli/src/opts.rs` | 9–40 |
| `Mount` CLI args (not gated) | `cli/src/opts.rs` | 217–254 |
| `Run` CLI args (not gated) | `cli/src/opts.rs` | 138–183 |
| `Exec` CLI args (Unix-only) | `cli/src/opts.rs` | 190 |
| `Nfs` legacy CLI args (Unix-only) | `cli/src/opts.rs` | 285–298 |
| `Serve::Nfs` CLI args (Unix-only) | `cli/src/opts.rs` | 376–389 |
| Per-OS `mount()` dispatch | `cli/src/cmd/mount.rs` | 57–83 |
| Unix-only `mount_fuse` | `cli/src/cmd/mount.rs` | 86– |
| NFS server module list | `cli/src/nfsserve/mod.rs` | 1–24 |
| Only Windows-gated module in NFS | `cli/src/nfsserve/mod.rs` | 19 |
| Vendored fuser code root | `cli/src/fuser/**` | — |
| FUSE mount syscall | `cli/src/fuser/mnt/fuse_pure.rs` | — |
| `cli/Cargo.toml` Linux deps | `cli/Cargo.toml` | 57–70 |
| `cli/Cargo.toml` macOS deps | `cli/Cargo.toml` | 72–77 |
| Sandbox feature flag | `cli/Cargo.toml` | 15–23 |
| Sandbox crate (Linux-only) | `sandbox/Cargo.toml`, `sandbox/src/lib.rs` | — |

## 8. References (external)

- WinFsp: https://winfsp.dev/ — the kernel driver + libfuse-compatible shim
- `winfsp-sys` crate: low-level Rust FFI to WinFsp's native C API
- `winfsp` crate: higher-level Rust wrapper (if it covers what you need)
- Microsoft docs — Client for NFS: search "Services for NFS Windows 10" — the optional Windows feature that provides `mount.exe`
- macOS `run_darwin.rs` is the closest in-repo template — read it before writing `run_windows.rs`
- tursodatabase/agentfs on GitHub — for filing the design discussion issue

---

## Appendix A — the experiments I ran before writing this

Verified upstream behavior on **agentfs v0.6.4** (binary at `E:\Tools\nerdies\bin\agentfs.exe`), Windows 10 Pro 19045, WinFsp installed (Launcher service running, no `HKLM:\SOFTWARE\WinFsp` reg key found — installer may put it under `WOW6432Node`):

```
> agentfs init test --base C:\Windows\TEMP\agentfs-test-...
Created overlay filesystem: .agentfs\test.db   ← worked

> agentfs mount test <mp> --backend fuse -f
Error: Mounting is only available on Unix (Linux or macOS)

> agentfs mount test <mp> --backend nfs -f
Error: Mounting is only available on Unix (Linux or macOS)

> agentfs run pwd
Error: The `run` command is not supported on Windows
```

Both `mount` errors fire **before** the backend is inspected — confirming the gate is at the module-routing level (`#[cfg(unix)]` in `cli/src/cmd/mod.rs`), not the backend dispatch.
