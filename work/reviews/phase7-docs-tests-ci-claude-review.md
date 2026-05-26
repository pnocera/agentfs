# Claude Review - Phase 7 Windows docs tests and CI review

## Verdict

GO_STATUS: GO

## Findings

- [low] .github/workflows/windows-rust.yml:25-27 - The CI installs `dtolnay/rust-toolchain@stable`, but `cli/rust-toolchain.toml` pins the cli crate to `channel = "nightly"`. When `cargo` runs from `working-directory: cli`, the toolchain file overrides the default; rustup will silently auto-install nightly for every CI run (slow, and may need to re-fetch clippy/rustfmt as nightly components). The `sdk` job has no `rust-toolchain.toml`, so it actually runs on stable as intended. Either switch the cli job to `dtolnay/rust-toolchain@nightly` (with `components: rustfmt, clippy` resolved for nightly) and update the cache key to include the toolchain channel, or remove `cli/rust-toolchain.toml` if the code does build on stable. Without a CI run on record yet, this is a likely "first PR breaks" trap; the local smoke is correct but uses a developer-installed nightly.
- [low] README.md:10 - The build-status badge still points at `https://github.com/tursodatabase/agentfs/actions/workflows/rust.yml`, but that workflow was deleted before Phase 0. Phase 7 adds `windows-rust.yml`. Badge will now show "no status" or 404. Swap to `windows-rust.yml` (and add Linux/macOS workflow badges if/when those return), or remove the badge until CI surface stabilizes.
- [low] cli/src/cmd/nfs.rs:113-118 - `resolve_db_path` has dead code where both arms of `if db_path.exists()` return the same value. Pre-existing, not Phase 7's fault — flagging because Phase 7 touched this file (added the line 103-104 comment about agent-ID format invariants). A one-line cleanup (just `Ok(db_path)`) would tidy.
- [low] .github/workflows/windows-rust.yml - No Linux/macOS jobs are restored. The earlier deletion of `.github/workflows/rust.yml` (pre-Phase-0) means non-Windows regressions won't be caught by CI going forward; only the Phase 7 ad-hoc local logs (`work/phase7-cargo-check-linux-cli-no-default-features.log`, `work/phase7-cargo-check-macos-cli-default-features.log`) backstop those platforms. Phase 7's plan acceptance only asks for Windows jobs, so this is out of scope, but the carry-over gap is worth flagging.
- [low] MANUAL.md:219 - The manual mount example shows `\\127.0.0.1@11111\!` (the Unix-default port). The Windows runtime in `cli/src/mount/nfs.rs` defaults to 2049 because the Windows Client for NFS doesn't accept high-port localhost forms reliably. The doc is technically correct for `agentfs serve nfs --port 11111`, but a reader copying the example for use with `agentfs mount` might be confused. Worth adding a sentence noting that `agentfs mount` selects port 2049 on Windows.
- [info] .github/workflows/windows-rust.yml:39-55 - The cli job runs `cargo fmt -- --check`, default-features check, no-default check, all-targets clippy, lib tests, and binary build — covering the Phase 7 plan acceptance lines `cargo fmt`, `cargo clippy`, `cargo test`. The order (fmt → check → clippy → test → build) is the right shape: fast lints first, slow builds last.
- [info] .github/workflows/windows-rust.yml:57-89 - The sdk job runs fmt-check, target-specific check, and test. Missing `clippy --all-targets` for SDK, which would catch the same benchmark drift the Phase 4 spike flagged. Not a Phase 7 regression — the SDK clippy gap pre-dates the new CI. Worth a one-line follow-up to add.
- [info] .github/workflows/windows-rust.yml:30-37 + 73-80 - Cache strategy: cli cache keyed on `cli/Cargo.lock + sdk/rust/Cargo.lock`, sdk cache keyed on `sdk/rust/Cargo.lock`. Reasonable separation. Cache `path:` values (`cli/target`, `sdk/rust/target`) are workspace-relative, which is correct for `actions/cache@v4`.
- [info] .github/workflows/windows-rust.yml:8 - `workflow_dispatch:` is enabled, allowing manual reruns. Good for the foreseeable need to retry after Phase 0-6 Windows NFS client issues are resolved.
- [info] cli/src/cmd/nfs.rs:121-151 - `print_windows_client_examples` now prints both cmd.exe and PowerShell mount.exe invocations, using the same `anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1` options string as the Phase 2/3 runtime probe at `cli/src/mount/nfs.rs:215`. Phase 6 v1 review's low-severity concern about the cmd.exe-only example and option drift is fully resolved. Confirmed in `work/phase7-cli-smoke-windows-serve-nfs.log:13-19`.
- [info] MANUAL.md:13-34 - Windows requirements section covers: Client for NFS optional feature with the exact `Enable-WindowsOptionalFeature` PowerShell commands; explicit note about Windows Home edition limits; expected System32 tool paths and the `NfsClnt` service; drive-letter mountpoints; "FUSE and WinFsp-native mounts are deferred" — not silently unsupported; `agentfs run` is "copy-on-write overlay execution only ... not a security sandbox" with the explicit `AGENTFS_SANDBOX=windows-overlay-only` marker. Matches the plan's "Document Windows requirements" task verbatim.
- [info] MANUAL.md:78-79 + 178-179 - `agentfs init -c` and `--backend fuse` Windows restrictions documented in line with command sections. `agentfs run` Windows constraints at line 136-137.
- [info] TESTING.md:1-54 - Windows section gives the exact Phase 7 verification commands plus the two-terminal manual smoke for live mount/run, with a Network Error 53 hint pointing at the local Client for NFS as the diagnostic. Matches the plan's "Manual smoke" checklist.
- [info] README.md:101-126 - Windows getting-started note: enable Windows Client for NFS, mount to an unassigned drive letter, two-terminal flow, and the explicit "Windows v1 is overlay-only copy-on-write execution and is not a security sandbox" sentence. Acceptably terse with a pointer to MANUAL.md for the full requirements.
- [info] work/phase7-cargo-test-windows-cli-lib.log:11-57 - 44 lib tests pass on Windows (unchanged from Phase 6 — Phase 7 didn't add tests, per the spike). Includes the four `cmd::nfs::tests`, the five `cmd::run::sys::tests`, the five `mount::nfs::tests`, and the `mount::tests::windows_mount_fs_rejects_fuse_backend` test — full Windows-specific coverage.
- [info] work/phase7-cargo-test-windows-sdk.log:1-10 - SDK test set still passes (76 tests + doctest, per spike claim). Phase 7 didn't modify SDK; verifies no regression.
- [info] work/phase7-cargo-clippy-windows-cli-all-targets.log:19-20 - Same two pre-existing clippy warnings carry over (`init.rs:109`, `ps.rs:219`). The Phase 5 `run_not_supported.rs:9` warning remains gone since Windows default-features uses `run_windows.rs`. `--all-targets` includes tests; warning count is unchanged from Phase 5/6.
- [info] work/phase7-cargo-fmt.log - Both `cli/Cargo.toml` and `sdk/rust/Cargo.toml` pass `cargo fmt -- --check`. Plan acceptance "cargo fmt --all" satisfied with the two manifest-local runs.
- [info] work/phase7-cli-smoke-windows-serve-nfs.log:13-26 - Runtime evidence that the new cmd.exe + PowerShell example output formats correctly for an ephemeral port; both forms include the `@PORT` and `:PORT` variants and omit the no-port `\\host\!` form (correct for a non-2049 port).
- [info] work/phase7-cargo-check-linux-cli-no-default-features.log + work/phase7-cargo-check-macos-cli-default-features.log - Linux no-default and macOS default-features checks both pass. Linux default-features check is intentionally not re-run because the Phase 5 environment gap (`libunwind-ptrace.pc` missing) is unchanged and unrelated to Phase 7.

## Required Fixes

- None

## Verification Notes

- Read all 16 subjects in full and cross-checked against `work/plan/implementation-plan.md:227-256` (Phase 7 documentation, unit-test, CI, and manual-smoke task lists + acceptance).
- Verified each plan task:
  - Documentation: README/MANUAL/TESTING all cover Client for NFS, edition/SKU limits, drive-letter mountpoints, v1 no-sandbox, FUSE/WinFsp deferral.
  - Unit tests: rely on Phase 1-6 coverage (drive-letter parser at `cli/src/mount/nfs.rs`, Windows NFS source forms at `cli/src/cmd/nfs.rs` and `cli/src/mount/nfs.rs`, device major/minor at `cli/src/nfs.rs` + `sdk/rust/src/filesystem/mod.rs`, HostFS basic ops at `sdk/rust/src/filesystem/hostfs_windows.rs`, run shell/drive helpers at `cli/src/cmd/run_windows.rs`). Test counts verified: 44 cli + 76 sdk.
  - CI: `windows-rust.yml` adds `windows-latest` cli + sdk jobs with fmt/check/clippy/test/build, no NFS optional feature install. Plan's "Keep integration tests gated on the NFS client feature being present" and "Do not make CI depend on kernel-driver installs for v1" both honored — the CI skips live mount/run smoke entirely.
  - Manual smoke: TESTING.md and MANUAL.md document the full two-terminal flow plus `agentfs init / fs write / mount / Get-Content / Set-Content / umount / fs cat / run / diff` sequence the plan calls out.
- Cross-checked the Phase 7 acceptance commands against the saved logs:
  - `cargo fmt --all` → `work/phase7-cargo-fmt.log` shows exit 0 for both manifest-local runs.
  - `cargo clippy -p agentfs --target x86_64-pc-windows-msvc --all-targets` → `work/phase7-cargo-clippy-windows-cli-all-targets.log` shows only the two pre-existing warnings.
  - `cargo test -p agentfs-sdk --target x86_64-pc-windows-msvc` → `work/phase7-cargo-test-windows-sdk.log` shows 76 tests passing.
  - `cargo test -p agentfs --target x86_64-pc-windows-msvc` → `work/phase7-cargo-test-windows-cli-lib.log` shows 44 lib tests passing.
  - Linux/macOS checks → both pass for the verified configurations.
- Inspected `cli/rust-toolchain.toml` to confirm the cli is pinned to nightly; the CI workflow's `@stable` then auto-falls-back to nightly inside `cli/` via rust-toolchain.toml resolution. Not strictly wrong, but worth tightening — see findings.
- Did not run cargo commands myself; relied on the 12 captured logs as instructed.
- Did not attempt to launch the CI workflow against a PR — first PR after merge will exercise the toolchain-resolution concern noted above.
