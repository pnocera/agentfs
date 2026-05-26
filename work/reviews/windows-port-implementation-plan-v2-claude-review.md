# Claude Review - Windows port implementation plan review v2

## Verdict

GO_STATUS: GO

## Findings

- None blocking. The v2 plan addresses every Required Fix from the v1 review with concrete, traceable changes. The remaining observations below are minor and can be handled during implementation without re-planning.
- [low] work/plan/implementation-plan.md:175 - Phase 5 picks an unused drive letter "from high to low (`Z:` to `D:`)" but does not describe how concurrent `agentfs run` invocations avoid both picking the same letter between the "is it free?" check and the `mount.exe` call. A simple "try-and-fall-back-on-error" loop is enough; just note it so the implementer doesn't write a naive check-then-mount that races.
- [low] work/plan/implementation-plan.md:150 - Phase 4's `FileSystem` task list includes `statfs` but does not name the Windows source. `GetDiskFreeSpaceExW` on the base directory is the obvious choice; a one-line nudge would prevent the implementer from inventing a synthetic value.
- [low] work/plan/implementation-plan.md:142-148 - Phase 4 specifies a "deterministic 63-bit fingerprint over volume + file id" but leaves the algorithm unspecified. That is reasonable for a plan, but please pick a fixed hash (e.g., SipHash-2-4 with a constant key, or `xxh3_64` masked to 63 bits) in the implementation PR so the mapping is reproducible across builds and reviewable.

## Required Fixes

- None

## Verification Notes

- Confirmed every Required Fix from `work/reviews/windows-port-implementation-plan-claude-review.md` is addressed in v2:
  - Phase 4 inode mapping: plan lines 142-148 spell out `FileIdInfo` preferred + `BY_HANDLE_FILE_INFORMATION` fallback, 63-bit fingerprint into `Stats.ino: i64`, reserved root, per-`HostFS` bidirectional cache, shared by `lookup`/`getattr`/`readdir_plus`/`open`/NFS `fileid3`.
  - Phase 2 `MountHandle::drop` cfg-gating: plan line 90 explicitly preserves the existing Unix `set_current_dir("/")` and scopes the `temp_dir()` change to a Windows branch only (current code at `cli/src/mount/mod.rs:114-117` would otherwise have been changed for all targets).
  - `get_mounts` / Windows prune: plan lines 204-210 make the V1 decision explicit — `agentfs_sdk::get_mounts()` stays Linux-only, Windows list/prune stay stubbed unless a CLI-owned registry is added first, and `mount.exe` output is explicitly forbidden as ownership signal.
  - Phase 0 acceptance: plan line 56 requires the working command line be saved to `work/windows-nfs-client-spike.md`; Phase 2 line 95 forbids guessing the non-default port syntax in code and references the spike artifact.
  - Phase 5 prompt customization: plan lines 177-180 add `/K` for `cmd.exe`, a `prompt` function for PowerShell, and env-var fallback for non-shell commands.
  - Phase 5 `AGENTFS_SANDBOX`: plan line 184 picks the concrete value `"windows-overlay-only"` with a documented "not a security sandbox" contract, matching the convention from `cli/src/sandbox/{darwin,linux}.rs`.
  - Current Checkout Facts path: plan line 27 now reads `cli/src/cmd/run_windows.rs`.
  - O_RDONLY/O_RDWR strategy: plan lines 68-72 pick the centralize-in-SDK option (constants live next to `FileSystem::open` in `sdk/rust/src/filesystem/mod.rs`, re-exported from `agentfs-sdk`, consumed by `cli/src/nfs.rs`).
  - Phase 1 clippy acceptance: plan line 78 adds `cargo clippy -p agentfs --target x86_64-pc-windows-msvc --no-default-features --all-targets`.
  - Phase 3 `mount_nfs_backend` refactor scope: plan line 120 explicitly calls out that the `exists()`/`canonicalize` refactor must happen inside the shared `mount_nfs_backend`, not just per-OS entry points.
  - Phase 6 dependency on Phase 1: plan line 201 notes the dependency on Phase 1's libc cleanup and lib.rs ungating.
- Spot-verified two source claims still hold against the worktree: `Stats.ino: i64` at `sdk/rust/src/filesystem/mod.rs:108`, and `cli/src/mount/mod.rs:117` still hardcodes `set_current_dir("/")` (the v2 plan's cfg-gate is required, not a no-op).
- Git context shows no plan-relevant source changes since v1 review (only deleted CI workflows and `.gitignore` tweaks). The plan still describes the current checkout accurately.
- `work/00-tooling-spike/notes.md` referenced in the prompt does not exist; treating as "no cached facts available" (same as v1 review).
