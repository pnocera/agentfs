//! Windows run command implementation.
//!
//! Windows v1 provides copy-on-write execution for the current working
//! directory through HostFS, OverlayFS, and the NFS mount backend. It is not a
//! security sandbox: paths outside the mounted working directory remain
//! accessible to the child process.

use agentfs_sdk::{AgentFS, AgentFSOptions, EncryptionConfig, FileSystem, HostFS, OverlayFS};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::mount::{mount_fs, MountBackend, MountOpts};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    allow: Vec<PathBuf>,
    no_default_allows: bool,
    experimental_sandbox: bool,
    strace: bool,
    session_id: Option<String>,
    system: bool,
    encryption: Option<(String, String)>,
    command: PathBuf,
    args: Vec<String>,
) -> Result<()> {
    validate_windows_run_options(
        &allow,
        no_default_allows,
        experimental_sandbox,
        strace,
        system,
    )?;

    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let home = dirs::home_dir().context("Failed to get home directory")?;
    let session = setup_run_directory(session_id, &home)?;

    let db_path_str = session
        .db_path
        .to_str()
        .context("Database path contains non-UTF8 characters")?;

    let encrypted = encryption.is_some();
    let mut options = AgentFSOptions::with_path(db_path_str);
    if let Some((key, cipher)) = encryption {
        options = options.with_encryption(EncryptionConfig {
            hex_key: key,
            cipher,
        });
    }
    let agentfs = AgentFS::open(options)
        .await
        .context("Failed to create delta AgentFS")?;

    let hostfs = HostFS::new(cwd.clone()).context("Failed to create Windows HostFS")?;
    let overlay = OverlayFS::new(Arc::new(hostfs), agentfs.fs);

    let cwd_str = cwd
        .to_str()
        .context("Current directory path contains non-UTF8 characters")?;
    overlay
        .init(cwd_str)
        .await
        .context("Failed to initialize overlay")?;
    std::fs::write(&session.base_path_file, cwd_str)
        .context("Failed to write session base path")?;

    let fs: Arc<Mutex<dyn FileSystem + Send>> = Arc::new(Mutex::new(overlay));
    let mount_opts = MountOpts {
        mountpoint: session.mount_root.clone(),
        backend: MountBackend::Nfs,
        fsname: format!("agentfs:{}", session.session_id),
        uid: None,
        gid: None,
        allow_other: false,
        allow_root: false,
        auto_unmount: false,
        lazy_unmount: true,
        timeout: std::time::Duration::from_secs(10),
    };

    let mount_handle = mount_fs(fs, mount_opts).await?;

    print_welcome_banner(&session, &cwd, encrypted);

    if let Err(e) =
        crate::cmd::ps::write_proc_file(&session.session_id, true, &command.to_string_lossy(), &cwd)
    {
        eprintln!("Warning: Failed to write proc file: {}", e);
    }

    let exit_code = match run_command_in_mount(&session, command, args).await {
        Ok(exit_code) => exit_code,
        Err(err) => {
            crate::cmd::ps::remove_proc_file(&session.session_id);
            return Err(err);
        }
    };

    crate::cmd::ps::remove_proc_file(&session.session_id);
    let procs_dir = crate::cmd::ps::procs_dir(&session.session_id);
    let _ = std::fs::remove_dir(&procs_dir);

    drop(mount_handle);

    eprintln!();
    eprintln!("Session: {}", session.session_id);
    eprintln!();
    eprintln!("To resume this session:");
    eprintln!("  agentfs run --session {}", session.session_id);
    eprintln!();
    eprintln!("To see what changed:");
    eprintln!("  agentfs diff {}", session.session_id);

    std::process::exit(exit_code);
}

fn validate_windows_run_options(
    allow: &[PathBuf],
    no_default_allows: bool,
    experimental_sandbox: bool,
    strace: bool,
    system: bool,
) -> Result<()> {
    if experimental_sandbox {
        bail!(
            "--experimental-sandbox is not supported on Windows. Windows v1 uses copy-on-write execution only and is not a security sandbox."
        );
    }
    if strace {
        bail!("--strace is not supported on Windows");
    }
    if !allow.is_empty() {
        eprintln!("Warning: --allow cannot be enforced on Windows v1; ignoring");
    }
    if no_default_allows {
        eprintln!("Warning: --no-default-allows has no effect on Windows v1; ignoring");
    }
    if system {
        eprintln!("Warning: --system has no effect on Windows v1; ignoring");
    }
    Ok(())
}

struct RunSession {
    session_id: String,
    run_dir: PathBuf,
    db_path: PathBuf,
    base_path_file: PathBuf,
    mount_root: PathBuf,
}

fn setup_run_directory(session_id: Option<String>, home: &Path) -> Result<RunSession> {
    let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let run_dir = home.join(".agentfs").join("run").join(&session_id);
    std::fs::create_dir_all(&run_dir).context("Failed to create run directory")?;

    let mount_root = find_unused_drive_letter()?;

    Ok(RunSession {
        session_id,
        db_path: run_dir.join("delta.db"),
        base_path_file: run_dir.join("base_path"),
        run_dir,
        mount_root,
    })
}

fn find_unused_drive_letter() -> Result<PathBuf> {
    find_unused_drive_letter_with(|root| !root.exists())
        .context("Could not find an unused drive letter from Z: through D:")
}

fn find_unused_drive_letter_with<F>(mut is_available: F) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    for letter in (b'D'..=b'Z').rev() {
        let root = drive_root(letter as char);
        if is_available(&root) {
            return Some(root);
        }
    }
    None
}

fn drive_root(letter: char) -> PathBuf {
    PathBuf::from(format!("{}:\\", letter.to_ascii_uppercase()))
}

fn print_welcome_banner(session: &RunSession, cwd: &Path, encrypted: bool) {
    eprintln!("Welcome to AgentFS!");
    eprintln!();
    eprintln!("Windows run mode:");
    eprintln!(
        "  - {} is mounted copy-on-write at {}",
        cwd.display(),
        session.mount_root.display()
    );
    eprintln!("  - This is not a security sandbox. Other filesystem paths remain accessible.");
    if encrypted {
        eprintln!("  - Delta layer is encrypted.");
    }
    eprintln!();
    eprintln!("Session data: {}", session.run_dir.display());
    eprintln!("Environment: AGENTFS=1, AGENTFS_SANDBOX=windows-overlay-only");
    eprintln!();
}

async fn run_command_in_mount(
    session: &RunSession,
    command: PathBuf,
    args: Vec<String>,
) -> Result<i32> {
    let (command, args) = prepare_windows_command(command, args);

    let mut child = Command::new(&command)
        .args(&args)
        .current_dir(&session.mount_root)
        .env("AGENTFS", "1")
        .env("AGENTFS_SANDBOX", "windows-overlay-only")
        .env("AGENTFS_SESSION", &session.session_id)
        .env("PROMPT", "[agentfs] $P$G")
        .spawn()
        .with_context(|| format!("Failed to execute command: {}", command.display()))?;

    let status = tokio::select! {
        status = child.wait() => status?,
        signal = tokio::signal::ctrl_c() => {
            signal.context("Failed to listen for Ctrl+C")?;
            eprintln!("Received Ctrl+C; terminating child process...");
            let _ = child.kill().await;
            child.wait().await?
        }
    };

    Ok(status.code().unwrap_or(1))
}

fn prepare_windows_command(command: PathBuf, args: Vec<String>) -> (PathBuf, Vec<String>) {
    if !args.is_empty() {
        return (command, args);
    }

    if command_name_is(&command, &["cmd", "cmd.exe"]) {
        return (
            command,
            vec!["/K".to_string(), "prompt [agentfs] $P$G".to_string()],
        );
    }

    if command_name_is(
        &command,
        &["powershell", "powershell.exe", "pwsh", "pwsh.exe"],
    ) {
        return (
            command,
            vec![
                "-NoExit".to_string(),
                "-Command".to_string(),
                "function prompt { \"[agentfs] $($executionContext.SessionState.Path.CurrentLocation)> \" }"
                    .to_string(),
            ],
        );
    }

    (command, args)
}

fn command_name_is(command: &Path, names: &[&str]) -> bool {
    let Some(name) = command.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    names
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_unused_drive_from_high_to_low() {
        let drive = find_unused_drive_letter_with(|root| root != Path::new(r"Z:\")).unwrap();
        assert_eq!(drive, PathBuf::from(r"Y:\"));
    }

    #[test]
    fn returns_none_when_no_drive_is_available() {
        assert!(find_unused_drive_letter_with(|_| false).is_none());
    }

    #[test]
    fn prepares_interactive_cmd_prompt() {
        let (command, args) = prepare_windows_command(PathBuf::from("cmd.exe"), Vec::new());
        assert_eq!(command, PathBuf::from("cmd.exe"));
        assert_eq!(args, vec!["/K", "prompt [agentfs] $P$G"]);
    }

    #[test]
    fn prepares_interactive_powershell_prompt() {
        let (command, args) = prepare_windows_command(PathBuf::from("pwsh.exe"), Vec::new());
        assert_eq!(command, PathBuf::from("pwsh.exe"));
        assert_eq!(args[0], "-NoExit");
        assert_eq!(args[1], "-Command");
        assert!(args[2].contains("[agentfs]"));
    }

    #[test]
    fn leaves_explicit_shell_args_unchanged() {
        let args = vec!["/c".to_string(), "echo hello".to_string()];
        let (command, prepared) = prepare_windows_command(PathBuf::from("cmd.exe"), args.clone());
        assert_eq!(command, PathBuf::from("cmd.exe"));
        assert_eq!(prepared, args);
    }
}
