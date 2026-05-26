# Windows NFS Client Port 111 Fix

Date: 2026-05-26

## Finding

The built-in Windows Client for NFS does not reliably use AgentFS's fake
portmapper when the AgentFS NFS listener is bound to port 2049 or a high
localhost port. With the listener on 2049, `agentfs mount` still fails with
`Network Error - 53` for all tested forms:

- `\\127.0.0.1\!`
- `\\127.0.0.1@2049\!`
- `\\127.0.0.1:2049\!`

When the same AgentFS NFS listener is bound to port 111, Windows `showmount`
can enumerate the root export and `mount.exe` can mount `\\127.0.0.1\!` to a
drive letter. The nfsserve listener already handles portmap, mount, and NFS RPC
on one TCP port; Windows expects to discover that combined service through the
standard portmapper port.

## Code Changes

- Changed the Windows `mount_fs` NFS default from port 2049 to port 111.
- Windows `agentfs mount` now fails clearly if localhost TCP port 111 is not
  available, because fallback ports are not known to work with the built-in
  Windows Client for NFS.
- Updated Windows NFS source-form helpers to try the no-port `\\host\!` form
  for both port 111 and port 2049.
- Updated Windows `agentfs serve nfs` examples and tests for the port 111
  no-port form.
- On Windows only, AgentFS reports permissive NFS access bits to clients
  (`0777` for directories and `0666` for other nodes) while leaving the
  underlying AgentFS mode unchanged. This keeps anonymous Windows NFS mounts
  writable without changing Linux/macOS NFS behavior.
- Documented the Windows port 111 requirement, permissive Windows NFS mode bits,
  and current `NFSPROC3_COMMIT` durability caveat in `MANUAL.md`.

## Verification

Evidence logs:

- `work/completion-audit-direct-mount.log`: current pre-fix `agentfs mount`
  with port 2049 fails at Windows Client for NFS `Network Error - 53`.
- `work/completion-audit-rpcinfo.log`: AgentFS listener on 2049 starts, but
  Windows `showmount` cannot contact the portmapper.
- `work/completion-audit-port111-mount-v2.log`: AgentFS listener on port 111
  is mountable through `\\127.0.0.1\!` and `Z:\hello.txt` can be read.

Final post-fix verification is captured in:

- `work/port111-fix-cargo-check-windows-cli-default-features.log`
- `work/port111-fix-cargo-check-windows-cli-no-default-features.log`
- `work/port111-fix-cargo-clippy-windows-cli-all-targets.log`
- `work/port111-fix-cargo-test-windows-cli-lib.log`
- `work/port111-fix-cargo-build-windows-cli.log`
- `work/port111-fix-cargo-test-windows-sdk.log`
- `work/port111-fix-cargo-check-linux-cli-no-default-features.log`
- `work/port111-fix-cargo-check-macos-cli-default-features.log`
- `work/port111-fix-cli-smoke-windows-direct-mount.log`
- `work/port111-fix-cli-smoke-windows-run.log`

The direct mount smoke proves:

- `agentfs mount <id> Z: --backend nfs -f` stays running.
- `Get-Content Z:\hello.txt` reads data written through `agentfs fs`.
- `Set-Content Z:\created.txt created` writes through the Windows NFS drive.
- `umount.exe Z:` succeeds.
- `agentfs fs <id> cat /created.txt` returns `created` after unmount.

The run smoke proves:

- `agentfs run --session port111_fix_run cmd.exe /c "echo hello>created.txt"`
  exits 0.
- The original working directory does not contain `created.txt`.
- `agentfs diff port111_fix_run` resolves the run-session delta and reports
  `A f /created.txt`.
