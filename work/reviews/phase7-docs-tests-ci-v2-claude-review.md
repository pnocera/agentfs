# Claude Review - Phase 7 Windows docs tests and CI review v2

## Verdict

GO_STATUS: GO

## Findings

- [info] .github/workflows/windows-rust.yml:25-37 - The cli job now uses `dtolnay/rust-toolchain@nightly` with `components: rustfmt, clippy`, and the cache key includes `cli/rust-toolchain.toml` so a toolchain channel bump invalidates the cache. This directly addresses the v1 low-severity finding about the cli being nightly-pinned via `cli/rust-toolchain.toml` while CI installed stable. The sdk job correctly stays on `@stable` since `sdk/rust` has no toolchain pin.
- [info] README.md:9-10 - Build-status badge now points at `windows-rust.yml` (both the workflow path and the badge image URL match). v1's stale `rust.yml` badge reference is resolved.
- [info] MANUAL.md:231-234 - Added the disambiguation block "agentfs mount starts its internal Windows NFS server at port 2049 when that port is available ... The 11111 examples above apply to `agentfs serve nfs --port 11111` and other explicitly chosen ports." This closes v1's low-severity confusion between the runtime probe port and the manual mount example port.
- [info] work/phase7-docs-tests-ci.md:34-35, 96-98 - Phase 7 doc now explicitly records the v2 changes: "The CLI job uses `dtolnay/rust-toolchain@nightly` because the CLI crate pins nightly in `cli/rust-toolchain.toml`. The SDK job uses stable." and the port-2049-vs-11111 disambiguation. The doc accurately reflects the CI configuration in the workflow file.
- [info] cli/src/cmd/nfs.rs:103-104 + 121-151 - Unchanged from v1: agent-ID `.db` heuristic comment present, dual cmd.exe/PowerShell example output with consistent `anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1` options. The Phase 6/Phase 7 evolution is preserved cleanly.
- [info] work/phase7-cli-smoke-windows-serve-nfs.log:13-19 - Runtime smoke evidence for the dual-shell example output is unchanged (still 2049-vs-non-2049 logic correctly applied to the ephemeral port 55868).
- [info] work/phase7-cargo-test-windows-cli-lib.log:11-57 + work/phase7-cargo-test-windows-sdk.log:1-10 - 44 cli lib tests + 76 sdk tests + 1 sdk doctest, all still passing. No regressions from the v1-to-v2 docs/CI tightening.
- [info] work/phase7-cargo-clippy-windows-cli-all-targets.log:19 - Same two pre-existing clippy warnings (`init.rs:109`, `ps.rs:219`); v2 doesn't introduce new clippy noise.
- [info] work/phase7-cargo-fmt.log + work/phase7-cargo-check-windows-cli-default-features.log + work/phase7-cargo-check-windows-cli-no-default-features.log + work/phase7-cargo-build-windows-cli.log - All four artifacts still exit 0; the v1-to-v2 changes were doc + workflow only, no code surface impact.
- [info] work/phase7-cargo-check-linux-cli-no-default-features.log + work/phase7-cargo-check-macos-cli-default-features.log - Linux no-default and macOS default-features checks both still pass. v2 didn't touch crate sources; cross-platform checks remain green where the build environment allows.
- [info] .github/workflows/windows-rust.yml:30-37 + 73-80 - Cache strategy: cli cache keyed on `cli/rust-toolchain.toml + cli/Cargo.lock + sdk/rust/Cargo.lock`, sdk cache keyed on `sdk/rust/Cargo.lock`. The cli cache rebuild on toolchain bumps is exactly what's needed when nightly cycles through breaking changes.
- [info] .github/workflows/windows-rust.yml:57-89 - The sdk job still runs `cargo fmt -- --check`, `cargo check`, `cargo test`. The v1 observation that SDK clippy `--all-targets` isn't covered by CI remains (acknowledged as a Phase 4 carry-over, the bench files use stale `OverlayFS` APIs). Not blocking — the `--lib --tests` clippy path covered the Phase 4 surface via local runs, and the CI Test step compiles the test binary which proves the active SDK surface still builds.

## Required Fixes

- None

## Verification Notes

- Re-read all 17 subjects in full and walked each v1 required-fix line against the v2 state:
  - v1 finding "CI installs stable but cli pins nightly" → v2 `windows-rust.yml:25-26` uses `@nightly` for cli, `@stable` for sdk; spike line 34-35 documents this. Resolved.
  - v1 finding "README badge points at deleted `rust.yml`" → v2 `README.md:10` points at `windows-rust.yml` with matching image URL. Resolved.
  - v1 finding "MANUAL.md example port 11111 could mislead `agentfs mount` users" → v2 `MANUAL.md:231-234` adds the port-2049-vs-11111 disambiguation. Resolved.
  - v1 finding "Linux/macOS CI not restored" → still open by design (Phase 7 scope is Windows jobs only); restated below as info.
  - v1 finding "resolve_db_path has dead code in exists() check" → still open; Phase 7 v2 didn't touch this. Pre-existing low-severity carry-over, not introduced or worsened by v2.
  - v1 finding "SDK `clippy --all-targets` not in CI" → still open by design (Phase 4 spike covers the bench API drift); not blocking.
- Verified the v2 workflow file syntactically and semantically:
  - `working-directory: cli` + `@nightly` action means `cargo` from `cli/` will use the action-installed nightly (with rustfmt + clippy components) instead of triggering an implicit rustup auto-download per CI run.
  - `working-directory: sdk/rust` + `@stable` means the sdk job actually runs on stable, which is correct given there's no `sdk/rust/rust-toolchain.toml`.
  - Cache key now invalidates correctly when `cli/rust-toolchain.toml` is bumped.
- Did not run cargo commands myself; relied on the 11 captured logs. All evidence is unchanged from v1 because v2 only touched docs and workflow YAML.
- Did not trigger an actual CI run — that will exercise on the first PR after merge. The v2 configuration matches the developer's local nightly toolchain, so the workflow should behave equivalently to the captured local runs.
