# Phase 6 - Secondary CLI Surfaces

Date: 2026-05-26

## Scope

Phase 6 exposes the standalone NFS server command on Windows after the core
Windows NFS and HostFS work from phases 1-5.

Implemented:

- `agentfs serve nfs` is now compiled on Windows.
- Legacy `agentfs nfs` is now compiled on Windows.
- Windows server startup output prints Windows Client for NFS mount examples.
- Direct database paths containing Windows backslashes are treated as paths, not
  agent IDs.
- Windows-specific tests cover wildcard bind host conversion and default versus
  non-default NFS source forms.

Deferred by v1 decision:

- `agentfs mount list` remains stubbed on Windows until an AgentFS-owned mount
  registry exists.
- `agentfs prune mounts` remains stubbed on Windows until that registry exists.
- `agentfs init -c` remains explicitly unsupported on Windows until temporary
  drive-letter mounting is promoted for init workflows.
- `agentfs exec` remains hidden on Windows until mount/run behavior is proven
  enough to reuse the temporary-drive flow safely.

## Changed Files

- `cli/src/cmd/mod.rs`: ungates the standalone NFS command module for Windows.
- `cli/src/opts.rs`: exposes `agentfs serve nfs` and legacy `agentfs nfs` in the
  Windows CLI.
- `cli/src/main.rs`: dispatches both NFS command surfaces on Windows.
- `cli/src/cmd/nfs.rs`: adds Windows mount guidance, fixes Windows path-looking
  DB arguments, and adds Windows-only unit tests.

## Windows Serve Smoke

`work/phase6-cli-smoke-windows-serve-nfs.log` starts:

```text
agentfs.exe serve nfs F:\Tools\agentfs\work\phase6-serve-smoke.db --bind 127.0.0.1 --port <ephemeral>
```

Result:

- The process reached `Listening: 127.0.0.1:<ephemeral>`.
- It printed Windows Client for NFS examples for both verified non-default-port
  source forms.
- It printed the Unix client example for non-Windows clients.
- The smoke stopped the server after startup verification; it did not attempt a
  live Windows NFS client mount because this machine still returns the known
  Client for NFS `Network Error - 53` for localhost exports.

## Verification

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo check --target x86_64-pc-windows-msvc` | PASS | `work/phase6-cargo-check-windows-cli-default-features.log` |
| `cargo test --target x86_64-pc-windows-msvc --lib` | PASS, 44 tests | `work/phase6-cargo-test-windows-cli-lib-default-features.log` |
| `cargo clippy --target x86_64-pc-windows-msvc` | PASS with pre-existing warnings in `init.rs` and `ps.rs` | `work/phase6-cargo-clippy-windows-cli-default-features.log` |
| `cargo build --target x86_64-pc-windows-msvc` | PASS | `work/phase6-cargo-build-windows-cli-default-features.log` |
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | PASS | `work/phase6-cargo-check-windows-cli-no-default-features.log` |
| Windows CLI help for `serve`, `serve nfs`, and legacy `nfs` | PASS | `work/phase6-cli-help-serve-nfs-windows.log` |
| Windows standalone NFS server startup smoke | PASS | `work/phase6-cli-smoke-windows-serve-nfs.log` |
| Linux no-default CLI check | PASS | `work/phase6-cargo-check-linux-cli-no-default-features.log` |
| macOS no-default CLI check | PASS | `work/phase6-cargo-check-macos-cli-no-default-features.log` |
| macOS default CLI check | PASS | `work/phase6-cargo-check-macos-cli-default-features.log` |
| `cargo fmt --manifest-path cli\Cargo.toml` | PASS | command completed successfully |
| `git diff --check` | PASS | command completed successfully |

## Notes

The Windows examples intentionally include both high-port source forms
documented during the Phase 0 client spike. For port 2049 the output also
includes the no-port `\\host\!` form.
