# Claude Review - Phase 3 Windows mount command integration review

## Verdict

GO_STATUS: GO

## Findings

- [low] cli/src/mount/nfs.rs:131 + 331-341 - On Windows, `find_available_port(DEFAULT_NFS_PORT)` returns the first available port ≥ 2049, but `windows_nfs_sources` only includes the no-port `\\127.0.0.1\!` form when `port == 2049`. If port 2049 is unavailable (e.g., a machine running the Windows NFS Server role), the function falls back to 2050+ and the probe set drops to just `@PORT` / `:PORT` — both of which the smoke test showed don't work locally either. Either pin to 2049 on Windows and fail loudly when unavailable, or document that fallback ports are effectively unreachable so the failure mode is intentional. Not blocking because the smoke evidence already shows none of the three forms work on this machine; the concern is purely about a hypothetical second user with port 2049 already bound.
- [low] work/phase3-windows-mount-command.md:49-67 - The smoke section documents that all three probe forms returned Error 53 from a running localhost AgentFS NFS server at port 2049, but doesn't capture a hypothesis. ERROR_BAD_NETPATH against a bound localhost listener is unusual and the next phase will need a starting point — adding one line (Windows redirector behavior with `\\127.0.0.1` UNCs, rpcbind/portmap expectations, or a known-incompatibility with the vendored `nfsserve` library) would speed the follow-up. Not a Phase 3 blocker since the artifact is honest about what it does and doesn't prove.
- [low] cli/src/mount/mod.rs:8-15 - The `ignore`d doctest example uses `MountBackend::Fuse` with `/mnt/agent`, which now errors on macOS and Windows. Doesn't break the build, but it's misleading documentation post-Phase-3 — a small follow-up should switch the example to `MountBackend::Nfs` or note the Linux-only nature.
- [info] cli/src/cmd/mod.rs:10-14 - `cmd::mount` is now `#[cfg(any(unix, target_os = "windows"))]` and `mount_stub.rs` is reserved for other targets. Matches the plan's task wording exactly; `mount_stub.rs` is no longer used on supported platforms but is correctly preserved for completeness.
- [info] cli/src/cmd/mount.rs:92-108 - Windows `mount()` bails *before* setup for both unsupported scenarios:
  - `MountBackend::Fuse` → "FUSE mounting is not supported on Windows. Use --backend nfs instead."
  - `MountBackend::Nfs` without `-f/--foreground` → "Windows NFS mounts must run in foreground mode (-f/--foreground) so the AgentFS NFS server remains alive."
  Both messages are actionable and match the plan's "explain the supported backend" / "If true daemonization is not implemented, print that clearly" acceptance criteria.
- [info] cli/src/cmd/mount.rs:227-296 - Shared `mount_nfs_backend` correctly cfg-gates mountpoint validation: Unix still runs `exists()` + `canonicalize`, Windows keeps the raw drive letter and defers validation to the NFS helper. Overlay configuration is detected once for both platforms; Windows bails with the plan's exact wording ("overlay-backed mounts require Windows HostFS support; direct AgentFS mounts are supported") *before* attempting to construct a `HostFS`, which is the right stopping point for Phase 4.
- [info] cli/src/cmd/mount.rs:298-326 - Foreground mode prints "Mounted at <drive>" and "Press Ctrl+C to unmount and exit." then awaits `ctrl_c()`. `lazy_unmount: true` is the right choice for Ctrl+C cleanup — on Windows it becomes `umount.exe -f`, on Linux it becomes `umount -l`; both let the cleanup proceed even with stale open handles.
- [info] cli/src/cmd/mount.rs:320-326 - Windows background NFS bail in `mount_nfs_backend` is defensive duplication of the early bail in `mount()`. Unreachable in practice but harmless; keeps the function's contract correct if called from a different entry point in a future phase.
- [info] cli/src/mount/nfs.rs:17-22 - `DEFAULT_NFS_PORT` is split: 11111 on non-Windows (unchanged), 2049 on Windows (new). The Windows choice is documented inline as the canonical NFS port the local client expects to be able to resolve via the no-port UNC form. Reasonable trade-off given the smoke evidence that high-port localhost UNCs are rejected.
- [info] cli/src/mount/nfs.rs:331-341 - `windows_nfs_sources` returns a `Vec<String>` (changed from `[String; 2]` in Phase 2) so the no-port form can be prepended conditionally for port 2049. Unit tests at `cli/src/mount/nfs.rs:421-431` and `:411-419` lock in both orderings.
- [info] cli/src/cmd/mount.rs:674-707 - Two new Windows tests cover the plan's explicit acceptance criteria for early rejection (`windows_mount_rejects_fuse_before_setup`, `windows_mount_rejects_background_nfs`). Combined with the existing mount tests, the Windows test count is now 8 — all passing per `work/phase3-cargo-test-windows-mount.log`.
- [info] work/phase3-cli-smoke-windows-mount-fuse.log - Real binary smoke: `agentfs.exe mount :memory: Z: --backend fuse` exits 1 with the plan's exact error message before any setup. Acceptance criterion #2 satisfied with binary evidence.
- [info] work/phase3-cli-smoke-windows-mount-nfs.log - Real binary smoke: `agentfs.exe mount :memory: Z: --backend nfs -f` reaches the Windows Client for NFS for all three configured probe forms, gets Error 53 for each, and the error message itemizes the per-form failure. Acceptance criterion #1 ("reaches the Windows NFS client") is satisfied — the plan does not require the mount itself to succeed at Phase 3. The artifact correctly defers the underlying ERROR_BAD_NETPATH puzzle to a later phase.
- [info] work/phase3-cargo-check-linux-cli-no-default-features.log + work/phase3-cargo-check-macos-cli-no-default-features.log - Linux and macOS compile checks both pass. The shared `mount_nfs_backend` refactor's Unix paths are byte-identical to Phase 2 behavior (validation, canonicalize, overlay HostFS construction), and the Linux/macOS `mount()` functions are untouched. Acceptance criterion #3 ("Linux/macOS command behavior is unchanged") satisfied.
- [info] work/phase3-cargo-test-linux-cli-mount.log:184-186 - Linux test run with `mount::` filter reports `0 passed; 0 failed; 0 ignored; 0 measured; 78 filtered out` — correct, because the new mount tests are `#[cfg(target_os = "windows")]`-only. The 78 filtered tests are the existing Linux CLI suite, which the build proves still compiles.
- [info] work/phase3-cargo-clippy-windows-no-default-features.log:2-32 - The same three pre-existing clippy warnings carry over from prior phases (`init.rs:109`, `ps.rs:219`, `run_not_supported.rs:9`). No new warnings introduced by Phase 3.

## Required Fixes

- None

## Verification Notes

- Read all 15 subjects in full and cross-checked against `work/plan/implementation-plan.md:106-129` (Phase 3 tasks + acceptance).
- Verified each plan task:
  - Reuse `cli/src/cmd/mount.rs` for Windows (no `mount_windows.rs` copy) — confirmed, the single `mount.rs` has Linux/macOS/Windows branches.
  - `cli/src/cmd/mod.rs` compiles `mount.rs` for `#[cfg(any(unix, target_os = "windows"))]` and keeps `mount_stub.rs` for other targets — confirmed at `cli/src/cmd/mod.rs:10-14`.
  - Windows `mount(args)` branch:
    - `MountBackend::Nfs` calls the existing NFS backend flow — confirmed at `cli/src/cmd/mount.rs:103-106`.
    - `MountBackend::Fuse` bails with a Windows-specific "use --backend nfs" message — confirmed at `cli/src/cmd/mount.rs:94-97`.
  - Mountpoint refactor inside the shared `mount_nfs_backend` — confirmed at `cli/src/cmd/mount.rs:227-235`.
  - Direct AgentFS databases supported, overlay returns a clear error on Windows — confirmed at `cli/src/cmd/mount.rs:276-296`.
  - Foreground/background behavior explicit on Windows — confirmed at `cli/src/cmd/mount.rs:98-102` and `:320-326`.
- Verified all three plan acceptance criteria:
  - `agentfs mount <id> Z: --backend nfs -f` reaches the Windows NFS client — `phase3-cli-smoke-windows-mount-nfs.log:5-14` shows the client returning per-form Network Error 53.
  - `agentfs mount <id> Z: --backend fuse` fails before setup — `phase3-cli-smoke-windows-mount-fuse.log:4` shows the bail with the right message and no prior setup output.
  - Linux/macOS unchanged — `phase3-cargo-check-linux-cli-no-default-features.log` and `phase3-cargo-check-macos-cli-no-default-features.log` both finish clean.
- Verified the Phase 2 v1 review's required Phase-2-handoff items are landed: Windows `MountHandle::drop` uses `temp_dir()` (`cli/src/mount/mod.rs:121-122`), and `mount_nfs` cancels + aborts + awaits the server task on `nfs_mount` failure (`cli/src/mount/nfs.rs:149-154`). Both arrived in Phase 2 and remain in place in Phase 3.
- Verified the smoke harness's `--- cleanup ---` output ("Drive Z is not an NFS mounted drive / umount exit code 39") is the smoke script's belt-and-braces post-step, not a `MountHandle::drop` warning — since `nfs_mount` returned Err, no `MountHandle` was constructed, so the Drop guard never fired.
- Did not run any cargo commands; relied on the 10 captured logs as instructed.
- Did not attempt to debug the underlying ERROR_BAD_NETPATH against the live AgentFS NFS server — the spike and Phase 3 doc both defer this to a later phase, and the plan's Phase 3 acceptance only requires the command path to reach the client.
