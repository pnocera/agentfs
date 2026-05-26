# Windows NFS Client Spike

Date: 2026-05-26
Repo: `F:\Tools\agentfs`
Phase: 0 baseline and client spike

## Summary

The Windows build baseline is clean when cargo runs from the `cli` crate in a proper MSVC environment:

- VS setup: `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`
- Extra compiler path: `C:\Program Files\LLVM\bin` for `clang-cl.exe`
- No-default check log: `work/phase0-cargo-check-windows-no-default-features.log`
- Default-feature check log: `work/phase0-cargo-check-windows-default-features.log`

Important caveat: this is the current pre-port Windows baseline. It does not prove the NFS mount path compiles on Windows yet, because `cli/src/lib.rs` still gates `nfsserve`, `nfs`, and `mount` behind `#[cfg(unix)]`. The meaningful NFS compile baseline is the Phase 1 acceptance check after those gates are widened.

The built-in Windows Client for NFS is now enabled and available locally. WinFsp is also installed, but this machine's WinFsp install does not include an NFS bridge executable; it provides the WinFsp driver, launcher, FUSE headers/libs, and samples.

## Toolchain Evidence

Everything search found multiple usable MSVC environments:

```text
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat
C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat
E:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat
E:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat
```

Selected Phase 0 build environment:

```cmd
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set PATH=C:\Program Files\LLVM\bin;%PATH%
```

`cl.exe`, `link.exe`, and `clang-cl.exe` are available in that environment.

## Cargo Baseline Results

Commands were run from `F:\Tools\agentfs\cli`, because this repository has crate-local Cargo manifests and no root `Cargo.toml`.
That is the crate-local equivalent of the plan's root-workspace-style `cargo check -p agentfs ...` command; `cli/Cargo.toml` declares package `agentfs`, and the logs show `Checking agentfs v0.6.4 (F:\Tools\agentfs\cli)`.

```cmd
cargo check --target x86_64-pc-windows-msvc --no-default-features
```

Result: pass, exit code 0. See `work/phase0-cargo-check-windows-no-default-features.log`.

```cmd
cargo check --target x86_64-pc-windows-msvc
```

Result: pass, exit code 0. See `work/phase0-cargo-check-windows-default-features.log`.

## Built-In Windows Client For NFS

Optional feature state:

```text
ServicesForNFS-ClientOnly: Enabled
ClientForNFS-Infrastructure: Enabled
NFS-Administration: Disabled
```

Installed tools:

```text
C:\Windows\System32\mount.exe
C:\Windows\System32\umount.exe
C:\Windows\System32\showmount.exe
C:\Windows\System32\nfsadmin.exe
C:\Windows\System32\rpcinfo.exe
```

Running service:

```text
NfsClnt / Client for NFS / Running / Auto / C:\Windows\system32\nfsclnt.exe
```

`mount.exe /?` reports this syntax:

```text
mount [-o options] [-u:username] [-p:<password | *>] <\\computername\sharename> <devicename | *>
```

Relevant supported options from local help:

```text
-o anon
-o nolock
-o casesensitive=yes|no
-o mtype=soft|hard
-o timeout=time
-o retry=number
-o sec=sys|krb5|krb5i|krb5p
```

`umount.exe /?` reports:

```text
umount.exe [-f] <-a | drive_letters | network_mounts>
```

## Non-Default Port Syntax Finding

The local Windows `mount.exe` help does not document `port=` or `mountport=` options and does not document a non-default port syntax. This matters because AgentFS's userspace NFS server binds to a high localhost port instead of privileged NFS ports.

Candidate forms were tested against localhost without a live AgentFS NFS server:

```text
\\127.0.0.1\!          -> Network Error 53
\\127.0.0.1@11111\!    -> Network Error 53
\\127.0.0.1:11111\!    -> Network Error 53
127.0.0.1:/            -> Network Error 53
127.0.0.1@11111:/      -> Network Error 53
127.0.0.1:11111:/      -> Network Error 67
```

Interpretation:

- `127.0.0.1:11111:/` produces a different failure (`Network Error 67`) than the other candidates (`Network Error 53`).
- Error codes alone do not conclusively distinguish syntactic acceptance from redirector-level name lookup failure, because both errors are produced before a live NFS exchange can complete.
- This does not prove which non-default port form works with a live NFS server.

The plan's requested "exact working command line" was not fully proven in Phase 0 because no live AgentFS NFS server was available on Windows to complete a mount. Phase 2 should therefore implement one of these safe strategies:

1. Prefer a verified live-server smoke before hardcoding a non-default port form.
2. Implement a small Windows mount-client probe that tries `\\127.0.0.1@PORT\!` and `\\127.0.0.1:PORT\!` against the live AgentFS NFS server, accepts the first successful mount, and records/logs which form worked.
3. If neither form succeeds, return a clear error saying the built-in Windows Client for NFS does not expose a supported non-default-port mount path on this machine.

Do not silently assume the report's `\\127.0.0.1@PORT\!` form is correct without live confirmation.

## Recommended Windows Mount Invocation For Phase 2

Initial implementation should use the built-in Windows Client for NFS when present:

```cmd
mount.exe -o anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1 <server-export-form> Z:
```

`timeout=8` matches the Windows Client for NFS default of 8 tenths of a second, avoiding an artificially aggressive 100 ms first-RPC timeout while still keeping failures bounded. `retry=1` is acceptable for the Phase 2 probe; production code may choose a higher retry count after smoke testing.

`casesensitive=yes` preserves AgentFS/Unix-style path semantics more closely than Windows default case-insensitive lookup. Phase 3 user-facing docs should call this out and can expose a future option if Windows-native behavior is preferred.

Unmount:

```cmd
umount.exe Z:
```

Preflight detection:

- Prefer `%SystemRoot%\System32\mount.exe` and `%SystemRoot%\System32\umount.exe`.
- Verify file metadata describes "Client for NFS export/share mount utility" / "Client for NFS export/share un-mount utility" when available, but do not rely on those English strings alone because localized Windows language packs may change them.
- Fallback signals: the binary is the one resolved from `%SystemRoot%\System32`, the `NfsClnt` service exists, and the Windows optional features `ServicesForNFS-ClientOnly` / `ClientForNFS-Infrastructure` are enabled.
- Reject `mount.exe` binaries found in Git Bash, MSYS, Cygwin, Go, or other tool directories.
- If missing, report that the Windows optional features `ServicesForNFS-ClientOnly` and `ClientForNFS-Infrastructure` are required.

## WinFsp Evidence

Installed product:

```text
WinFsp 2025 2.1.25156
Publisher: Navimatics LLC
```

Running service:

```text
WinFsp.Launcher / Running / Auto
```

Installed files under `C:\Program Files (x86)\WinFsp` include:

- `inc\fuse\*.h`
- `inc\fuse3\*.h`
- `inc\winfsp\*.h`
- `lib\winfsp-x64.lib`
- `SxS\...\bin\winfsp-x64.dll`
- `SxS\...\bin\launcher-x64.exe`
- `SxS\...\bin\memfs-x64.exe`
- samples including `memfs`, `passthrough`, and FUSE samples

No local WinFsp NFS bridge executable was found. WinFsp remains relevant for a later native WinFsp backend or libfuse-shim backend, but it is not a drop-in replacement for the Phase 1-3 NFS client path.

## Phase 0 Conclusion

Phase 0 build prerequisites are partially satisfied:

- MSVC/LLVM toolchain exists and cargo Windows checks pass.
- Windows Client for NFS is enabled and discoverable.
- WinFsp is installed but not directly useful for the NFS-first path.

The plan's live "exact working command line" acceptance is not fully met because there was no live Windows AgentFS NFS server to complete a non-default-port mount during this spike. That confirmation is explicitly deferred to the Phase 2 runtime probe strategy above. Code should treat the non-default port form as a runtime probe or explicit unsupported-client error, not a known constant.
