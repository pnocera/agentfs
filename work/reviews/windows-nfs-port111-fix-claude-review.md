# Claude Review - Windows NFS port 111 live acceptance fix review

## Verdict

GO_STATUS: GO

## Findings

- [low] cli/src/mount/nfs.rs:171-182 + 333-344 - `find_available_port(111)` falls back to ports 112..211 if 111 is busy. `windows_nfs_sources` only emits the no-port `\\host\!` form for ports 111 and 2049, so a fallback port lands AgentFS on a configuration the Windows Client for NFS rejected throughout Phases 0-7 (Error 53 for `@PORT`/`:PORT` forms). The most likely "port 111 busy" scenario is a machine with Microsoft's NFS Server role enabled (which owns the system portmapper) — exactly the audience for `agentfs mount`. Either narrow the fallback to ports with a known-working source form, fail loud when 111 can't be bound on Windows, or document that the user must free port 111. Today the failure mode degrades silently back to the pre-fix Error 53.
- [low] cli/src/nfs.rs:83-86 + 570-577 - `windows_client_mode` clamps reported mode bits to `0o777`/`0o666` for Windows NFS clients while leaving `Stats.mode` unchanged. This is a sensible v1 trade-off (Windows Client for NFS uses anonymous UNIX credentials), but it does mean a file stored with `0o400` appears writable through the Windows drive — a real semantic divergence from Linux/macOS NFS mounts on the same AgentFS database. Worth (a) one targeted unit test that exercises the directory vs file branch and preserves upper bits, and (b) a one-line note in MANUAL.md so users aren't surprised that read-only modes don't enforce through Windows NFS.
- [low] MANUAL.md:226-235 - Updated to call out port 111 as the Windows default but doesn't mention:
  - The permissive-mode-bits behavior introduced by `windows_client_mode` (so users can't predict that `0o400` doesn't restrict Windows clients).
  - The conflict with Microsoft's NFS Server role's portmapper if both Windows NFS features are enabled.
  - The `Unimplemented message NFSPROC3_COMMIT` warning visible in the smoke logs (write-durability gap under `mtype=soft`).
  These three caveats are likely to come up in support; documenting them alongside the new port choice avoids a follow-up doc PR.
- [low] work/port111-fix-cli-smoke-windows-direct-mount.log:31 + work/port111-fix-cli-smoke-windows-run.log:14 - `Unimplemented message NFSPROC3_COMMIT` in both smoke runs. The vendored `nfsserve` doesn't implement COMMIT, so the AgentFS NFS server acknowledges the RPC without actually flushing data to durable storage. Combined with `-o mtype=soft`, a Windows client crash or network glitch can lose buffered writes the client treats as committed. Not introduced by this fix (the warning predates it), but the live acceptance evidence makes the gap actionable for the first time. Worth a tracker for "implement COMMIT or document the durability contract" and a one-line warning in MANUAL.md.
- [info] cli/src/mount/nfs.rs:17-25 - `DEFAULT_NFS_PORT` is split: Linux/macOS stay at 11111 (unprivileged), Windows uses 111 (the standard portmapper port). The Windows branch's rustdoc explains the rationale: "Our NFS listener handles portmap, mount, and NFS RPC on the same TCP port, so Windows needs that listener on 111 for drive mounts." Tight, accurate, and the right cross-platform scoping.
- [info] cli/src/mount/nfs.rs:333-344 + 424-446 - `windows_nfs_sources` now emits the no-port form for `port == 111 || port == 2049`. Three unit tests (`windows_portmapper_port_tries_no_port_source_first`, `default_nfs_port_tries_no_port_source_first`, `builds_non_default_port_probe_sources`) lock in the three port classes. Consistent with the example output in `cli/src/cmd/nfs.rs`.
- [info] cli/src/cmd/nfs.rs:159-162 + 181-191 - `print_windows_client_examples`'s source-form logic mirrors the runtime probe; both renamed tests (`windows_sources_include_no_port_form_for_portmapper_port`, `windows_sources_include_no_port_form_for_default_nfs_port`) keep doc and runtime in sync.
- [info] cli/src/cmd/fs.rs:198-298 - `resolve_diff_options` falls back to `~/.agentfs/run/<session>/delta.db` when `AgentFSOptions::resolve` can't find the id. This is what makes `agentfs diff <session>` work after a Phase 5 `agentfs run --session <id>` — the Phase 5 smoke (Run section) demonstrates the diff successfully resolving the session's delta DB. Platform-agnostic via `dirs::home_dir()`; works on Linux/macOS too.
- [info] cli/src/nfs.rs:570-577 - `windows_client_mode` preserves the upper mode bits via `(mode & !0o777) | access_bits`, so setuid/setgid/sticky aren't masked when present. Directory `0o777` vs file `0o666` branch matches POSIX default permissions for anonymous-credential clients.
- [info] work/completion-audit-direct-mount.log + work/completion-audit-rpcinfo.log + work/completion-audit-port111-mount-v2.log - Three audit logs cleanly diagnose the root cause: (a) pre-fix port 2049 fails with Error 53 in every source-form, (b) `rpcinfo -p 127.0.0.1` and `showmount -e 127.0.0.1` both report "can't contact portmapper" because the AgentFS NFS service was on 2049 instead of 111, (c) moving the listener to 111 makes both `showmount` and `mount.exe \\127.0.0.1\!` succeed. Strong causal narrative.
- [info] work/port111-fix-cli-smoke-windows-direct-mount.log:1-21 - End-to-end direct mount smoke succeeds: `mount_started_state=running`, `mounted=True`, `read_text=hello` (Get-Content Z:\hello.txt after `agentfs fs write`), `created_text_on_drive=created` (Set-Content Z:\created.txt through the Windows NFS drive), umount succeeds, post-unmount `agentfs fs cat /created.txt` returns `created`. Phase 3's acceptance criterion ("`agentfs mount <id> Z: --backend nfs -f` reaches the Windows NFS client and writes are persistent") now passes live.
- [info] work/port111-fix-cli-smoke-windows-run.log:1-31 - End-to-end run smoke succeeds: `run_exit=0`, `base_created_exists=False` (proving the write went to the overlay delta, not the host CWD), and `agentfs diff` reports `A f /created.txt`. Phase 5's acceptance criterion ("`agentfs run cmd.exe /c "echo hello>created.txt"` writes into the AgentFS delta layer, not the original working tree") now passes live.
- [info] work/port111-fix-cargo-test-windows-cli-lib.log:53 - 46 cli lib tests pass (up from 44 in Phase 7). The two new tests are the renamed/added port-111 source-form coverage.
- [info] work/port111-fix-cargo-test-windows-sdk.log:1-4 + work/port111-fix-cargo-clippy-windows-cli-all-targets.log:19-20 - 76 SDK tests still pass; same two pre-existing clippy warnings (`init.rs:109`, `ps.rs:219`). No regression.
- [info] work/port111-fix-cargo-check-linux-cli-no-default-features.log + work/port111-fix-cargo-check-macos-cli-default-features.log + work/port111-fix-cargo-check-windows-cli-default-features.log + work/port111-fix-cargo-check-windows-cli-no-default-features.log + work/port111-fix-cargo-build-windows-cli.log - All cross-platform compile/check matrices pass. The `windows_client_mode` function and `DEFAULT_NFS_PORT=111` are `#[cfg(target_os = "windows")]`-only, so Linux/macOS are unaffected.

## Required Fixes

- None

## Verification Notes

- Read all 20 subjects in full and cross-checked against `work/plan/implementation-plan.md`'s Phase 3 acceptance ("`agentfs mount <id> Z: --backend nfs -f` reaches the Windows NFS client") and Phase 5 acceptance ("`agentfs run cmd.exe /c "echo hello>created.txt"` writes into the AgentFS delta layer, not the original working tree"). Both are now satisfied with binary evidence, closing the long-running carry-over flagged in every Phase 3-7 review.
- Walked the audit narrative end-to-end:
  - `completion-audit-direct-mount.log`: shows that with `DEFAULT_NFS_PORT=2049` the Windows client returns Error 53 for all three probed forms. Establishes the pre-fix failure.
  - `completion-audit-rpcinfo.log`: shows `rpcinfo -p 127.0.0.1` and `showmount -e 127.0.0.1` both fail to contact the portmapper when the listener is on 2049 — the diagnostic that points to "use port 111."
  - `completion-audit-port111-mount-v2.log`: shows `mount.exe \\127.0.0.1\! Z:` succeeding when the AgentFS listener is bound to 111. Establishes the fix.
- Verified the source changes match the documented spike:
  - `cli/src/mount/nfs.rs:24-25`: Windows DEFAULT_NFS_PORT = 111.
  - `cli/src/mount/nfs.rs:333-344`: no-port form for 111 or 2049.
  - `cli/src/cmd/nfs.rs:159-162`: example output for no-port form when port is 111 or 2049.
  - `cli/src/nfs.rs:83-86 + 570-577`: Windows-only `windows_client_mode` returning permissive bits.
  - `cli/src/cmd/fs.rs:198-298`: `resolve_diff_options` fallback for run-session delta.
  - `MANUAL.md:231-235`: port 111 callout.
- Verified the live smokes line up with what the code does: foreground mount stays running, `Z:` shows hello.txt via Get-Content, Set-Content lands on Z: and survives unmount as seen via `agentfs fs cat`, run smoke writes to the overlay delta and `diff` shows the new entry.
- Did not run any cargo commands; relied on the 10 captured logs as instructed.
- Did not attempt to test the port-111-busy scenario (no Microsoft NFS Server role installed on this machine). The single-low-severity finding above is the theoretical concern; the captured environment has port 111 available.
