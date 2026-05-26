//! NFS backend implementation for the mount infrastructure.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Output;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::nfs::AgentNFS;
use crate::nfsserve::tcp::NFSTcp;

use super::{MountBackend, MountHandle, MountHandleInner, MountOpts};

/// Default NFS port to try (use a high port to avoid needing root).
#[cfg(not(target_os = "windows"))]
const DEFAULT_NFS_PORT: u32 = 11111;
/// Default Windows Client for NFS port.
///
/// The built-in Windows client discovers MOUNT/NFS through the standard
/// portmapper port. Our NFS listener handles portmap, mount, and NFS RPC on the
/// same TCP port, so Windows needs that listener on 111 for drive mounts.
#[cfg(target_os = "windows")]
const DEFAULT_NFS_PORT: u32 = 111;

#[cfg(target_os = "windows")]
struct WindowsNfsClient {
    mount: std::path::PathBuf,
    umount: std::path::PathBuf,
}

/// NFS unmount implementation (Linux).
#[cfg(target_os = "linux")]
pub(super) fn unmount_nfs(mountpoint: &Path, lazy: bool) -> Result<()> {
    let output = if lazy {
        Command::new("umount")
            .arg("-l")
            .arg(mountpoint)
            .output()
            .context("Failed to execute umount")?
    } else {
        Command::new("umount")
            .arg(mountpoint)
            .output()
            .context("Failed to execute umount")?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !lazy {
            let output2 = Command::new("umount").arg("-l").arg(mountpoint).output()?;
            if output2.status.success() {
                return Ok(());
            }
        }
        anyhow::bail!(
            "Failed to unmount: {}. You may need to manually unmount with: umount -l {}",
            stderr.trim(),
            mountpoint.display()
        );
    }

    Ok(())
}

/// NFS unmount implementation (macOS).
#[cfg(target_os = "macos")]
pub(super) fn unmount_nfs(mountpoint: &Path, lazy: bool) -> Result<()> {
    let _ = lazy;
    let output = Command::new("/sbin/umount")
        .arg(mountpoint)
        .output()
        .context("Failed to execute umount")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let output2 = Command::new("/sbin/umount")
            .arg("-f")
            .arg(mountpoint)
            .output()?;

        if !output2.status.success() {
            anyhow::bail!(
                "Failed to unmount: {}. You may need to manually unmount with: umount -f {}",
                stderr.trim(),
                mountpoint.display()
            );
        }
    }

    Ok(())
}

/// NFS unmount implementation (Windows).
#[cfg(target_os = "windows")]
pub(super) fn unmount_nfs(mountpoint: &Path, lazy: bool) -> Result<()> {
    let client = find_windows_nfs_client()?;
    let drive = parse_windows_drive_mountpoint(mountpoint)?;

    let mut command = Command::new(&client.umount);
    if lazy {
        command.arg("-f");
    }
    let output = command.arg(&drive).output().with_context(|| {
        format!(
            "Failed to execute Windows NFS unmount command: {}",
            client.umount.display()
        )
    })?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to unmount Windows NFS drive {}: {}. You may need to manually unmount with: {} {}",
            drive,
            output_summary(&output),
            client.umount.display(),
            drive
        );
    }

    Ok(())
}

/// Internal NFS mount implementation.
pub(super) async fn mount_nfs(
    fs: Arc<Mutex<dyn agentfs_sdk::FileSystem + Send>>,
    opts: MountOpts,
) -> Result<MountHandle> {
    use tokio_util::sync::CancellationToken;

    let nfs = AgentNFS::new(fs);

    #[cfg(target_os = "windows")]
    let port = find_required_windows_port(DEFAULT_NFS_PORT)?;
    #[cfg(not(target_os = "windows"))]
    let port = find_available_port(DEFAULT_NFS_PORT)?;

    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = crate::nfsserve::tcp::NFSTcpListener::bind(&bind_addr, nfs)
        .await
        .context("Failed to bind NFS server")?;

    // CancellationToken is kept for API compatibility, but the vendored nfsserve
    // doesn't support graceful shutdown. The task will be aborted on drop.
    let shutdown = CancellationToken::new();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = listener.handle_forever().await {
            eprintln!("NFS server error: {}", e);
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if let Err(err) = nfs_mount(port, &opts.mountpoint) {
        shutdown.cancel();
        server_handle.abort();
        let _ = server_handle.await;
        return Err(err);
    }

    Ok(MountHandle {
        mountpoint: opts.mountpoint,
        backend: MountBackend::Nfs,
        lazy_unmount: opts.lazy_unmount,
        inner: MountHandleInner::Nfs {
            shutdown,
            _server_handle: server_handle,
        },
    })
}

/// Find an available TCP port starting from the given port.
#[cfg(not(target_os = "windows"))]
fn find_available_port(start_port: u32) -> Result<u32> {
    for port in start_port..start_port + 100 {
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!(
        "Could not find an available port in range {}-{}",
        start_port,
        start_port + 100
    );
}

#[cfg(target_os = "windows")]
fn find_required_windows_port(port: u32) -> Result<u32> {
    if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
        return Ok(port);
    }

    anyhow::bail!(
        "Windows AgentFS NFS mounts require localhost TCP port {} for the built-in Client for NFS portmapper lookup. Free that port, disable the conflicting NFS server/portmapper service, or use `agentfs serve nfs --port <PORT>` for server-only access.",
        port
    );
}

/// Mount the NFS filesystem (Linux version).
#[cfg(target_os = "linux")]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("mount")
        .args([
            "-t",
            "nfs",
            "-o",
            &format!(
                "vers=3,tcp,port={},mountport={},nolock,soft,timeo=10,retrans=2",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to mount NFS: {}. Make sure NFS client tools are installed.",
            stderr.trim()
        );
    }

    Ok(())
}

/// Mount the NFS filesystem (Windows).
#[cfg(target_os = "windows")]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let client = find_windows_nfs_client()?;
    let drive = parse_windows_drive_mountpoint(mountpoint)?;
    let options = "anon,nolock,casesensitive=yes,mtype=soft,timeout=8,retry=1";

    let mut failures = Vec::new();
    for source in windows_nfs_sources(port) {
        let output = Command::new(&client.mount)
            .args(["-o", options])
            .arg(&source)
            .arg(&drive)
            .output()
            .with_context(|| {
                format!(
                    "Failed to execute Windows NFS mount command: {}",
                    client.mount.display()
                )
            })?;

        if output.status.success() {
            return Ok(());
        }

        failures.push(format!("{} -> {}", source, output_summary(&output)));
    }

    anyhow::bail!(
        "Failed to mount AgentFS NFS on Windows drive {}. The built-in Windows Client for NFS did not accept the supported non-default-port forms for localhost port {}. Tried:\n{}",
        drive,
        port,
        failures.join("\n")
    )
}

/// Mount the NFS filesystem (macOS version).
#[cfg(target_os = "macos")]
fn nfs_mount(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("/sbin/mount_nfs")
        .args([
            "-o",
            &format!(
                "locallocks,vers=3,tcp,port={},mountport={},soft,timeo=10,retrans=2",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount_nfs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to mount NFS: {}", stderr.trim());
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn find_windows_nfs_client() -> Result<WindowsNfsClient> {
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let mount = windows_system32_tool(&system_root, "mount.exe");
    let umount = windows_system32_tool(&system_root, "umount.exe");

    if !mount.is_file() || !umount.is_file() || !windows_nfs_service_exists(&system_root) {
        anyhow::bail!(
            "Windows Client for NFS is required for AgentFS NFS mounts. Enable the optional features ServicesForNFS-ClientOnly and ClientForNFS-Infrastructure, then retry. Expected tools: {} and {}",
            mount.display(),
            umount.display()
        );
    }

    Ok(WindowsNfsClient { mount, umount })
}

#[cfg(target_os = "windows")]
fn windows_system32_tool(system_root: &Path, name: &str) -> std::path::PathBuf {
    system_root.join("System32").join(name)
}

#[cfg(target_os = "windows")]
fn windows_nfs_service_exists(system_root: &Path) -> bool {
    let sc = windows_system32_tool(system_root, "sc.exe");
    Command::new(sc)
        .args(["query", "NfsClnt"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn parse_windows_drive_mountpoint(mountpoint: &Path) -> Result<String> {
    let raw = mountpoint.as_os_str().to_string_lossy();
    let trimmed = raw.trim_end_matches(['\\', '/']);
    let mut chars = trimmed.chars();

    let Some(letter) = chars.next() else {
        anyhow::bail!("Windows NFS mountpoint must be an unassigned drive letter like Z: or Z:\\");
    };
    let Some(colon) = chars.next() else {
        anyhow::bail!(
            "Windows NFS mountpoint must be an unassigned drive letter like Z: or Z:\\; got {}",
            mountpoint.display()
        );
    };

    if chars.next().is_none() && colon == ':' && letter.is_ascii_alphabetic() {
        return Ok(format!("{}:", letter.to_ascii_uppercase()));
    }

    anyhow::bail!(
        "Windows NFS mountpoint must be an unassigned drive letter like Z: or Z:\\; got {}",
        mountpoint.display()
    )
}

#[cfg(target_os = "windows")]
fn windows_nfs_sources(port: u32) -> Vec<String> {
    let mut sources = Vec::new();
    if port == 111 || port == 2049 {
        sources.push(r"\\127.0.0.1\!".to_string());
    }
    sources.extend([
        format!(r"\\127.0.0.1@{}\!", port),
        format!(r"\\127.0.0.1:{}\!", port),
    ]);
    sources
}

#[cfg(target_os = "windows")]
fn output_summary(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit status {}", output.status),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("stdout: {}; stderr: {}", stdout, stderr),
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_drive_mountpoints() {
        assert_eq!(
            parse_windows_drive_mountpoint(Path::new(r"Z:")).unwrap(),
            "Z:"
        );
        assert_eq!(
            parse_windows_drive_mountpoint(Path::new(r"z:\")).unwrap(),
            "Z:"
        );
        assert_eq!(
            parse_windows_drive_mountpoint(Path::new(r"Y:/")).unwrap(),
            "Y:"
        );
    }

    #[test]
    fn rejects_non_drive_mountpoints() {
        for input in [
            r"C:\agentfs",
            r"Z:\agentfs",
            r"agentfs",
            r".\Z:",
            r"1:",
            r"ZZ:",
            r":",
        ] {
            assert!(
                parse_windows_drive_mountpoint(Path::new(input)).is_err(),
                "expected {input} to be rejected"
            );
        }
    }

    #[test]
    fn builds_windows_nfs_client_paths_from_system_root() {
        let root = Path::new(r"C:\Windows");

        assert_eq!(
            windows_system32_tool(root, "mount.exe"),
            std::path::PathBuf::from(r"C:\Windows\System32\mount.exe")
        );
        assert_eq!(
            windows_system32_tool(root, "umount.exe"),
            std::path::PathBuf::from(r"C:\Windows\System32\umount.exe")
        );
    }

    #[test]
    fn builds_non_default_port_probe_sources() {
        assert_eq!(
            windows_nfs_sources(11111),
            vec![
                r"\\127.0.0.1@11111\!".to_string(),
                r"\\127.0.0.1:11111\!".to_string()
            ]
        );
    }

    #[test]
    fn windows_portmapper_port_tries_no_port_source_first() {
        assert_eq!(
            windows_nfs_sources(111),
            vec![
                r"\\127.0.0.1\!".to_string(),
                r"\\127.0.0.1@111\!".to_string(),
                r"\\127.0.0.1:111\!".to_string()
            ]
        );
    }

    #[test]
    fn default_nfs_port_tries_no_port_source_first() {
        assert_eq!(
            windows_nfs_sources(2049),
            vec![
                r"\\127.0.0.1\!".to_string(),
                r"\\127.0.0.1@2049\!".to_string(),
                r"\\127.0.0.1:2049\!".to_string()
            ]
        );
    }
}
