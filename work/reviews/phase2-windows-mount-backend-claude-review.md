# Claude Review - Phase 2 Windows NFS mount backend implementation review

## Verdict

GO_STATUS: GO

## Findings

- [low] cli/src/mount/nfs.rs:118-160 - The new cleanup-on-failure branch (`shutdown.cancel(); server_handle.abort(); let _ = server_handle.await;`) is a Phase 2 behavior change for **all** platforms, not just Windows — `mount_nfs` is shared and was previously `nfs_mount(port, &opts.mountpoint)?` on Linux/macOS too, which silently leaked the spawned server task on mount failure. The new path is strictly additive (success path is byte-identical) so the regression risk is low, but the Phase 2 spike doesn't capture a Linux/macOS check confirming this. Worth noting in `work/phase2-windows-mount-backend.md` so future readers don't think the cleanup is Windows-only.
- [low] cli/src/mount/nfs.rs:290-297 - `windows_nfs_service_exists` only verifies that `sc.exe query NfsClnt` *succeeds*; that returns success when the service exists in any state (running, stopped, disabled). A stopped/disabled `NfsClnt` still passes preflight but fails at `mount.exe` time with a less-informative error. The spike's accepted fallback says "the NfsClnt service exists" so this matches spec, but parsing `STATE:` from the output and warning when not `RUNNING` would shorten the support loop. Low-severity carry-over; not blocking.
- [low] cli/src/mount/nfs.rs:142-149 - The pre-mount sleep is still a fixed `100ms`, inherited from the Linux/macOS path. Combined with the Windows `timeout=8` (= 800 ms) NFS option and `retry=1`, a slow CI box could see the first probe fire before the listener has accepted the first connection. The two-form probe loop gives the listener another window, but Phase 3 live smoke will be the real test. Not actionable in Phase 2.
- [info] cli/src/mount/nfs.rs:206-239 - `nfs_mount` Windows probe iterates the two spike-documented forms (`\\127.0.0.1@PORT\!`, `\\127.0.0.1:PORT\!`) and accumulates per-form failures into a single actionable error containing drive, port, and the `output_summary` of each attempt. Correctly avoids hardcoding a single non-default-port form, matching the spike's deferred-confirmation strategy.
- [info] cli/src/mount/nfs.rs:265-282 - Preflight uses the spike's *fallback* signals (`%SystemRoot%\System32` path resolution + `NfsClnt` service existence) rather than the language-pack-fragile file-metadata description match. This is the correct read of the spike, which explicitly warned not to rely on English description strings.
- [info] cli/src/mount/nfs.rs:300-323 - `parse_windows_drive_mountpoint` trims trailing `\` / `/`, then requires exactly `<letter><':'>` with `letter.is_ascii_alphabetic()`. The eight tested inputs (`Z:`, `z:\`, `Y:/`, `C:\agentfs`, `Z:\agentfs`, `agentfs`, `.\Z:`, `1:`, `ZZ:`, `:`) hit every branch the plan called out. Output is normalized to uppercase, which gives mount.exe a canonical target.
- [info] cli/src/mount/nfs.rs:326-331 - `windows_nfs_sources` returns both port-form variants as a `[String; 2]` array, keeping the probe order explicit and unit-testable. Matches the spike's documented probe forms.
- [info] cli/src/mount/nfs.rs:89-115 - Windows `unmount_nfs` maps `lazy=true` to `umount.exe -f` (force). The umount.exe help in the spike shows `-f` is the force flag; "lazy" semantics (detach-even-if-busy) and "force" map closely enough that this is acceptable. Error message guides the user to the manual recovery command.
- [info] cli/src/mount/mod.rs:117-155 - `Drop` ordering is correct: pick Windows-appropriate cwd (`temp_dir()` vs `"/"`), cancel the cooperative shutdown token, run unmount (which lets the NFS client complete its umount RPC dance against the still-running server), then `_server_handle.abort()`. Aborting *before* unmount would have made the umount RPC fail; the chosen order is right.
- [info] cli/src/mount/mod.rs:204-215 - Windows `mount_fs` rejects `MountBackend::Fuse` with an actionable Windows-specific message ("Use --backend nfs instead"). The test at `cli/src/mount/mod.rs:267-281` constructs `MountOpts::new("Z:", Fuse)` and asserts the message substring; the test log confirms it passes.
- [info] cli/src/mount/mod.rs:98-107 - `MountHandleInner` no longer carries the v1-noted `#[cfg_attr(target_os = "windows", allow(dead_code))]`. With Windows `mount_fs` now constructing the `Nfs` variant, the variant is reachable on all supported platforms and the attribute would have been misleading. Correct removal.
- [info] work/phase2-cargo-test-windows-mount.log:10-17 - 5 mount-related tests pass on Windows (`builds_windows_nfs_client_paths_from_system_root`, `builds_non_default_port_probe_sources`, `parses_windows_drive_mountpoints`, `rejects_non_drive_mountpoints`, `windows_mount_fs_rejects_fuse_backend`); 28 other CLI tests are correctly filtered out by the `mount::` test filter.
- [info] work/phase2-cargo-clippy-windows-no-default-features.log:4-37 - Same three pre-existing clippy warnings as Phase 0/Phase 1 (`init.rs:109`, `ps.rs:219`, `run_not_supported.rs:9`); no new warnings introduced by Phase 2 changes.
- [info] work/phase2-windows-mount-backend.md - Spike claims map cleanly to the code: Windows `mount_fs` ✓, split `Drop` cwd cleanup ✓, server-task lifecycle ✓, preflight via System32 + NfsClnt ✓, drive-letter validation ✓, two-form probe ✓, Phase 0 recommended mount options (`anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1`) ✓, `umount.exe -f` for lazy ✓. The "No live Windows NFS mount smoke was run in Phase 2" note correctly defers live confirmation to Phase 3 per the spike's deferred-confirmation contract.

## Required Fixes

- None

## Verification Notes

- Read all 8 subjects in full and cross-referenced against `work/plan/implementation-plan.md:86-104` (Phase 2 tasks + acceptance) and `work/windows-nfs-client-spike.md` (probe forms, mount options, preflight strategy).
- Walked the Phase 2 acceptance checklist:
  - `mount_fs(..., MountBackend::Nfs)` compiles on Windows — `work/phase2-cargo-check-windows-no-default-features.log` and `…-default-features.log` both exit 0.
  - `mount_fs(..., MountBackend::Fuse)` bails on Windows with a Windows-specific message — `cli/src/mount/mod.rs:210-212` plus the `windows_mount_fs_rejects_fuse_backend` test at `cli/src/mount/mod.rs:267-281` (passing per test log).
  - Parser tests cover valid drive forms, assigned-looking directory paths, relative paths, and malformed drive strings — `parses_windows_drive_mountpoints` and `rejects_non_drive_mountpoints` at `cli/src/mount/nfs.rs:352-384` (passing per test log).
- Traced `parse_windows_drive_mountpoint` through every test input manually:
  - Valid: `Z:` → "Z:", `z:\` → "Z:", `Y:/` → "Y:".
  - Rejected: `C:\agentfs` (extra chars after colon), `Z:\agentfs` (extra chars after colon), `agentfs` (no colon at position 2), `.\Z:` (`.` not alphabetic), `1:` (`1` not alphabetic), `ZZ:` (second char is `Z` not `:`), `:` (no second char).
  All branches behave as the test asserts.
- Verified the Windows `Drop` cwd target (`std::env::temp_dir()`) and order of `cancel → unmount → abort` against the v1 Phase 1 review's required Phase 2 handoff notes. Both are implemented.
- Verified the probe forms in `windows_nfs_sources` exactly match the spike's documented candidates (`work/windows-nfs-client-spike.md:131`).
- Did not run cargo myself; relied on the four captured logs as instructed.
- No live Windows NFS smoke was attempted — Phase 2 explicitly defers this to Phase 3, which is consistent with the spike's runtime-probe strategy.
