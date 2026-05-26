# Claude Review - Phase 6 Windows secondary CLI surfaces review

## Verdict

GO_STATUS: GO

## Findings

- [low] cli/src/cmd/nfs.rs:127 - The Windows mount example uses cmd.exe-style `%SystemRoot%\System32\mount.exe` which doesn't expand in PowerShell (which needs `$env:SystemRoot`). Users running the example from PowerShell will copy-paste a non-expanding path. Either add a parallel PowerShell example or use a plain `mount.exe` and rely on `%SystemRoot%\System32` being on `PATH`. Not blocking — the cmd.exe form works for the documented `cmd.exe` default shell.
- [low] cli/src/cmd/nfs.rs:127 - The example shows `-o anon,nolock,casesensitive=yes,mtype=soft` but the runtime probe at `cli/src/mount/nfs.rs:215` uses `anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1`. Slight drift between documentation and code defaults. Minor: a user who runs the example verbatim gets the Phase 0-recommended options without the aggressive timeout, which is fine for manual debugging but inconsistent with what `agentfs mount` does internally.
- [low] cli/src/cmd/nfs.rs:104 - `resolve_db_path` now treats any string ending in `.db` as a direct path on every platform (previously only `/`-containing strings were paths). Agent IDs can't contain `.` per `AgentFSOptions::validate_agent_id` (alphanumeric + `-` + `_`), so this isn't a regression for legitimate agent IDs; worth a one-line comment so a future change to validate_agent_id doesn't silently break the new heuristic.
- [low] cli/src/cmd/mod.rs:22-24 + cli/src/opts.rs:191 - `agentfs exec` is still `#[cfg(unix)]`, so on Windows it appears as "unrecognized subcommand" rather than the friendlier "not supported on Windows" message the other deferred surfaces produce. The Phase 6 spike documents this as a v1 decision ("remains hidden on Windows"), but a hidden subcommand is a UX papercut next to the explicit bails from `mount list`, `prune mounts`, and `init -c`. Not blocking; matches the documented deferral.
- [info] cli/src/cmd/nfs.rs:66-72 + 120-141 - `handle_nfs_command` gates client-side example output on `target_os`: Windows prints the System32 mount.exe variant via `print_windows_client_examples`, non-Windows keeps the original Unix-style example. Plan requirement "Update printed client mount examples with Windows-specific guidance when running on Windows" satisfied.
- [info] cli/src/cmd/nfs.rs:151-162 - `windows_nfs_sources` mirrors the runtime probe order at `cli/src/mount/nfs.rs:331-341` exactly: no-port form first for 2049, then `@PORT` and `:PORT`; non-default ports omit the no-port form. Three unit tests (`windows_example_host_uses_localhost_for_wildcard_binds`, `windows_sources_include_no_port_form_for_default_port`, `windows_sources_only_include_explicit_port_forms_for_high_port`) lock in the wildcard host conversion and both port-form orderings — these are the "Windows-specific tests" the spike promises.
- [info] cli/src/cmd/nfs.rs:143-149 - `windows_example_host` converts `0.0.0.0`, `::`, `[::]` to `127.0.0.1` so the printed example points the Windows client at a reachable address. Reasonable user-facing transformation; tested.
- [info] cli/src/cmd/nfs.rs:99-118 - `resolve_db_path` now treats `\\` (Windows path separator) and a `.db` suffix as path indicators, in addition to `/`. The smoke at `work/phase6-cli-smoke-windows-serve-nfs.log` confirms `F:\Tools\agentfs\work\phase6-serve-smoke.db` is treated as a direct path on Windows. Plan task "Direct database paths containing Windows backslashes are treated as paths, not agent IDs" satisfied.
- [info] cli/src/cmd/mod.rs:18-20, cli/src/opts.rs:286-299 + 377-390, cli/src/main.rs:262-296 - All three layers (module gate, CLI surface, dispatcher) consistently widen `agentfs nfs` and `agentfs serve nfs` to `#[cfg(any(unix, target_os = "windows"))]`. No stale Unix-only gate remaining for either surface.
- [info] cli/src/cmd/init.rs:263-272 - `agentfs init -c` on Windows bails with the explicit message `The -c option is not supported on Windows`. The v1 deferral is implemented as an explicit error rather than silent fallthrough.
- [info] cli/src/cmd/mount.rs (Phase 3 carry-over) - `agentfs mount` list on Windows prints `Mount listing is not available on Windows yet.` (`list_mounts` Windows branch) and `agentfs prune mounts` bails with `Mount pruning is not available on Windows yet`. Plan's "Unsupported secondary surfaces fail explicitly" satisfied for these too.
- [info] cli/src/cmd/nfs.rs:138-140 - The Windows client example ends with the actionable hint: `If Windows returns Network Error 53, verify the Client for NFS can mount localhost exports on this machine.` Useful given the persistent Phase 0-3 Network Error 53 on this machine; the user is pointed at the right diagnostic.
- [info] work/phase6-cargo-test-windows-cli-lib-default-features.log:5-53 - 44 lib tests pass (was 41 in Phase 5). The three new tests in `cmd::nfs::tests` are visible at lines 16-19 of the log; existing 41 Phase 5 tests still pass with no regression.
- [info] work/phase6-cli-help-serve-nfs-windows.log - `agentfs serve --help`, `agentfs serve nfs --help`, and `agentfs nfs --help` all return useful output on Windows. Legacy `agentfs nfs` keeps its `--bind`/`--port` flags identical to `serve nfs` so prior scripts continue to work.
- [info] work/phase6-cli-smoke-windows-serve-nfs.log - Server reached `Listening: 127.0.0.1:55559`, printed both non-default-port Windows source forms (no-port form correctly omitted for port 55559), printed the Unix client example, and printed the Error 53 hint. Plan acceptance "Supported secondary surfaces have Windows smoke tests" satisfied. Live Windows client mount is intentionally not attempted since the same Phase 0-3 Network Error 53 still applies — out of Phase 6 scope.
- [info] work/phase6-cargo-clippy-windows-cli-default-features.log:19 - Same two pre-existing clippy warnings (`init.rs:109`, `ps.rs:219`); no new warnings from Phase 6 code.
- [info] work/phase6-cargo-check-linux-cli-no-default-features.log + work/phase6-cargo-check-macos-cli-no-default-features.log + work/phase6-cargo-check-macos-cli-default-features.log - All three cross-platform check matrices pass; Phase 6 gate changes don't regress Linux or macOS builds. Linux default-features check is intentionally omitted because the libunwind-ptrace environment issue from Phase 5 persists in this WSL and is unrelated to Phase 6 changes.

## Required Fixes

- None

## Verification Notes

- Read all 15 subjects in full and cross-checked against `work/plan/implementation-plan.md:196-222` (Phase 6 tasks + acceptance).
- Walked the Phase 6 task list against the code:
  - `agentfs serve nfs` / legacy `agentfs nfs`:
    - Ungate `opts.rs` (Command::Nfs at line 286, ServeCommand::Nfs at line 377), `main.rs` (line 262 and 288), `cmd/mod.rs` (line 19) ✓.
    - Update Windows client mount examples ✓ (`print_windows_client_examples`).
  - `agentfs mount` list / prune mounts: stubbed with explicit "not available on Windows yet" messages (verified in Phase 3 review and confirmed unchanged).
  - `agentfs init -c`: explicit `The -c option is not supported on Windows` bail at `cli/src/cmd/init.rs:263-272`.
  - `agentfs exec`: `#[cfg(unix)]` in both opts.rs (line 191) and cmd/mod.rs (line 23) — hidden on Windows, matches the v1 deferral.
- Verified the Phase 6 plan acceptance:
  - "Unsupported secondary surfaces fail explicitly" — `mount list` prints a clear message, `prune mounts` bails with a clear message, `init -c` bails with a clear message, `exec` is hidden (less ideal but documented).
  - "Supported secondary surfaces have Windows smoke tests" — `work/phase6-cli-smoke-windows-serve-nfs.log` shows server reaching the Listening state, plus help output for both `serve nfs` and legacy `nfs` at `work/phase6-cli-help-serve-nfs-windows.log`.
- Cross-checked the Windows example output in `work/phase6-cli-smoke-windows-serve-nfs.log:13-20` against `windows_nfs_sources(127.0.0.1, 55559)` and `print_windows_client_examples`: port 55559 ≠ 2049 → no `\\127.0.0.1\!` line; one `@55559` and one `:55559` line; correct.
- Did not run cargo commands myself; relied on the 10 captured logs as instructed.
- Did not attempt to debug the persistent Windows Client for NFS Network Error 53 — Phase 6's scope is the secondary CLI surfaces, not the Phase 0-3 client incompatibility.
