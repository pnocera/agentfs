# Testing AgentFS

## Windows

The Windows port is covered by compile, unit, and startup smoke checks that do
not require kernel-driver installs in CI.

```powershell
cd cli
cargo check --target x86_64-pc-windows-msvc
cargo check --target x86_64-pc-windows-msvc --no-default-features
cargo clippy --target x86_64-pc-windows-msvc --all-targets
cargo test --target x86_64-pc-windows-msvc --lib
cargo build --target x86_64-pc-windows-msvc

cd ..\sdk\rust
cargo test --target x86_64-pc-windows-msvc
```

Manual mount and run smoke tests require the Windows Client for NFS optional
feature and an unassigned drive letter:

```powershell
cd cli
cargo build --target x86_64-pc-windows-msvc
$agentfs = "$PWD\target\x86_64-pc-windows-msvc\debug\agentfs.exe"

& $agentfs init win-direct --force
& $agentfs fs win-direct write /hello.txt hello
```

Start the foreground mount in one terminal and leave it running:

```powershell
& $agentfs mount win-direct Z: --backend nfs -f
```

Use a second terminal for drive operations and unmounting:

```powershell
cd cli
$agentfs = "$PWD\target\x86_64-pc-windows-msvc\debug\agentfs.exe"
Get-Content Z:\hello.txt
Set-Content Z:\created.txt created
& $env:SystemRoot\System32\umount.exe Z:
& $agentfs fs win-direct cat /created.txt

& $agentfs run --session win-run cmd.exe /c "echo hello>created.txt"
& $agentfs diff win-run
```

If the Windows client returns `Network Error 53`, first verify that the local
Client for NFS can mount localhost exports. The CI workflow intentionally does
not run this live mount smoke.

## pjdfstest

```bash
git clone git@github.com:pjd/pjdfstest.git
cd pjdfstest
autoreconf -ifs
./configure
make pjdfstest
sudo make install
sudo dnf install perl-Test-Harness
mkdir -p ../agentfs-testing
cd ../agentfs-testing
agentfs init testing
mkdir mnt
sudo su
agentfs mount testing ./mnt
cd mnt
prove -rv ../../pjdfstest/tests/ 2>&1 | tee /tmp/pjdfstest.log
```

## xftests

First, build the `agentfs` executable and install it locally including the `mount.fuse.agentfs` helper:

```bash
cd cli
cargo build --release
cp target/release/agentfs /usr/local/bin
cp scripts/mount.fuse.agentfs /sbin
```

Then, clone the xfstests repo:

```bash
git clone git://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git
```

Configure the filesystem under test:

```bash
cat local.config
export FSTYP=fuse
export FUSE_SUBTYP=.agentfs
export TEST_DEV=<database file>
export TEST_DIR=<mount directory>
```

Then, run xfstests:

```bash
sudo ./check -g quick generic/
```
