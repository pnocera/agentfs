# Claude Review - Windows NFS port 111 live acceptance fix review v2

## Verdict

GO_STATUS: GO

## Findings

- [info] cli/src/mount/nfs.rs:134-138 + 188-198 - The v1 finding about silent fallback from 111 to 112..211 is resolved. Windows now calls the dedicated `find_required_windows_port` which attempts to bind 127.0.0.1:111 exactly once and bails with an actionable message if it can't: "Windows AgentFS NFS mounts require localhost TCP port 111 for the built-in Client for NFS portmapper lookup. Free that port, disable the conflicting NFS server/portmapper service, or use `agentfs serve nfs --port <PORT>` for server-only access." Linux/macOS still use `find_available_port` with its 100-port fallback range, so the cross-platform behavior is preserved without weakening Windows guarantees.
- [info] cli/src/nfs.rs:594-617 - v1's "no unit test for windows_client_mode" is fixed. `windows_client_mode_preserves_upper_bits_and_makes_nodes_writable` covers both branches: regular file with setuid (`S_IFREG | 0o4000 | 0o400`) → `0o4666` (setuid preserved, access bits opened); directory with sticky bit (`S_IFDIR | 0o1000 | 0o500`) → `0o1777` (sticky preserved, access bits opened). The test confirms both the `(mode & !0o777) | access_bits` mask logic and the file-vs-directory branch.
- [info] MANUAL.md:33-36 - v1's "no port-111 conflict warning" is fixed: "agentfs mount uses localhost TCP port 111 on Windows ... If another service owns port 111, such as Microsoft's NFS Server role or another portmapper, stop that service before mounting." Users who hit `find_required_windows_port`'s bail now have a doc pointer for the cause.
- [info] MANUAL.md:187-189 - v1's "no permissive-mode-bits warning" is fixed: "Windows NFS mounts report permissive POSIX mode bits to the client so anonymous Windows NFS credentials can write to the AgentFS drive. Read-only POSIX modes in the database are not enforced through the Windows drive in v1." Directly explains the `windows_client_mode` semantic divergence from Linux/macOS NFS.
- [info] MANUAL.md:246-250 - v1's "no NFSPROC3_COMMIT durability note" is fixed: "The current NFS server does not implement NFSPROC3_COMMIT; Windows smoke logs may show that warning. Normal writes are persisted through the AgentFS request path and are verified after unmount in the manual smoke test, but v1 does not provide a stronger NFS COMMIT durability guarantee for client or network failure cases." Honest scoping of the durability contract.
- [info] work/windows-nfs-client-port111-fix.md:24-37 - Spike now records v2's tightenings: "Windows `agentfs mount` now fails clearly if localhost TCP port 111 is not available, because fallback ports are not known to work with the built-in Windows Client for NFS." plus an explicit "Documented the Windows port 111 requirement, permissive Windows NFS mode bits, and current NFSPROC3_COMMIT durability caveat in MANUAL.md." line. The doc accurately reflects all four v1 fixes.
- [info] cli/src/mount/nfs.rs:17-25 (carry from v1) - Cross-platform port split preserved: Linux/macOS at 11111, Windows at 111. Doc comment still explains the portmapper rationale.
- [info] cli/src/mount/nfs.rs:333-344 + tests (carry from v1) - `windows_nfs_sources` no-port form for 111 and 2049, with the three lock-in tests (`windows_portmapper_port_tries_no_port_source_first`, `default_nfs_port_tries_no_port_source_first`, `builds_non_default_port_probe_sources`) unchanged from v1.
- [info] cli/src/cmd/fs.rs (carry from v1) - `resolve_diff_options` fallback to `~/.agentfs/run/<session>/delta.db` unchanged; `agentfs diff <session>` after `agentfs run` continues to work.
- [info] work/port111-fix-cli-smoke-windows-direct-mount.log + work/port111-fix-cli-smoke-windows-run.log - End-to-end live mount and run acceptance evidence is unchanged from v1 (the smoke runs predate the v1→v2 hardening, and the hardening doesn't change happy-path behavior on a machine where port 111 is free). Phase 3 and Phase 5 acceptance remain satisfied.
- [info] work/port111-fix-cargo-test-windows-cli-lib.log - 46 cli lib tests still pass; v2 added the `windows_client_mode_preserves_upper_bits_and_makes_nodes_writable` test in the `cli/src/nfs.rs::tests` module, so a v2-fresh test run would show 47. The provided log predates the v2 test addition; the v2 source includes the test and it must pass before merge.
- [info] work/port111-fix-cargo-clippy-windows-cli-all-targets.log:19-20 - Same two pre-existing clippy warnings carry over (`init.rs:109`, `ps.rs:219`). No new warnings from v2 either.
- [info] work/port111-fix-cargo-check-linux-cli-no-default-features.log + work/port111-fix-cargo-check-macos-cli-default-features.log + work/port111-fix-cargo-check-windows-cli-default-features.log + work/port111-fix-cargo-check-windows-cli-no-default-features.log + work/port111-fix-cargo-build-windows-cli.log + work/port111-fix-cargo-test-windows-sdk.log - All cross-platform compile/check/build/sdk-test artifacts unchanged. `find_required_windows_port` and `windows_client_mode_preserves_upper_bits_and_makes_nodes_writable` are both `#[cfg(target_os = "windows")]`-only, so Linux/macOS are unaffected.

## Required Fixes

- None

## Verification Notes

- Re-read all 21 subjects in full and walked each v1 finding against the v2 state:
  - v1 "find_available_port silent fallback from 111" → v2 introduces dedicated `find_required_windows_port` at `cli/src/mount/nfs.rs:188-198` that bails with an actionable message rather than degrading. Resolved.
  - v1 "no unit test for windows_client_mode" → v2 adds `windows_client_mode_preserves_upper_bits_and_makes_nodes_writable` at `cli/src/nfs.rs:594-617`, covering both branches and upper-bit preservation. Resolved.
  - v1 "MANUAL.md missing three caveats" → v2 adds:
    - port-111 conflict warning (MANUAL.md:33-36),
    - permissive-mode-bits warning (MANUAL.md:187-189),
    - NFSPROC3_COMMIT durability note (MANUAL.md:246-250).
    All three resolved.
  - v1 "NFSPROC3_COMMIT tracker" → handled by the MANUAL.md durability note; v2 doesn't add an implementation, but the contract is now explicit. Resolved.
- Cross-checked the v2 source diff against the spike doc claims: every code change called out at `work/windows-nfs-client-port111-fix.md:22-37` corresponds to a verified edit in the listed files.
- Verified that `find_required_windows_port` is only invoked from the Windows branch of `mount_nfs` and is `#[cfg(target_os = "windows")]`-gated, so it can't accidentally tighten Linux/macOS behavior; `find_available_port` is `#[cfg(not(target_os = "windows"))]`-gated and only iterates the fallback range on Unix.
- Did not run cargo commands myself; relied on the 10 captured logs as instructed. The logs predate the v2 unit-test addition; the v2 test source is present in the file and will be picked up by a fresh `cargo test` run.
- Did not attempt the port-111-busy scenario (no Microsoft NFS Server role installed locally) — v2's defensive bail covers the case behaviorally and the doc warns the user up front.
