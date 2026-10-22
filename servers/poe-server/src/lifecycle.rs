//! How the process ends: graceful stop, and the three ways a restart brings the server back.
//!
//! An admin's stop or restart request (web API, `poe_web_api::Lifecycle`) or a SIGINT/SIGTERM
//! reaches [`main`](crate) as a [`ShutdownKind`]. After the sessions are drained, [`decide`]
//! picks the [`Action`] and [`perform`] carries it out. See `docs/guides/server-admin.md`.

use std::{ffi::OsString, path::PathBuf, time::Duration};

use poe_config::{RESTART_EXIT_CODE, RestartMode};
use poe_web_api::ShutdownKind;

/// How long the sessions get to end (and save) after the kick.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Environment variable of a restarted child on platforms without `exec`: milliseconds to wait
/// before binding, for the old process (the parent) to release its ports.
pub const RESTART_WAIT_ENV: &str = "POE_SERVER_RESTART_WAIT_MS";

/// What the finished shutdown does with the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Replace the process image with a fresh start (Unix `exec`): same pid, arguments,
    /// environment and working directory; every socket is closed by the exec.
    Exec,
    /// Start a new process with the same arguments, then exit 0 (platforms without `exec`).
    Spawn,
    /// Exit with this code.
    Exit(i32),
}

/// A stop exits with 0; a restart follows `[server] restart_mode` and the platform.
pub fn decide(kind: ShutdownKind, mode: RestartMode, has_exec: bool) -> Action {
    match (kind, mode) {
        (ShutdownKind::Stop, _) => Action::Exit(0),
        (ShutdownKind::Restart, RestartMode::Exit) => Action::Exit(RESTART_EXIT_CODE),
        (ShutdownKind::Restart, RestartMode::Exec) if has_exec => Action::Exec,
        (ShutdownKind::Restart, RestartMode::Exec) => Action::Spawn,
    }
}

/// The path to start again. Linux reports a binary replaced since the start (a rebuild or an
/// upgrade) as `<path> (deleted)`: the new file sits at the plain path.
pub fn restart_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if !exe.exists()
        && let Some(plain) = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)"))
    {
        return Ok(PathBuf::from(plain));
    }
    Ok(exe)
}

/// Carries `action` out. Does not return, except when it could not restart and fell back to
/// the exit code of a supervised restart.
pub fn perform(action: Action) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match action {
        Action::Exit(code) => std::process::exit(code),
        Action::Exec => {
            tracing::info!("restarting: exec");
            match exec(&args) {
                Ok(never) => match never {},
                Err(e) => {
                    tracing::error!("restart: exec failed: {e}");
                    std::process::exit(RESTART_EXIT_CODE)
                }
            }
        }
        Action::Spawn => {
            let spawned = restart_exe().and_then(|exe| {
                std::process::Command::new(exe)
                    .args(&args)
                    .env(RESTART_WAIT_ENV, "2000")
                    .spawn()
            });
            match spawned {
                Ok(child) => {
                    tracing::info!(pid = child.id(), "restarting: started a new process");
                    std::process::exit(0)
                }
                Err(e) => {
                    tracing::error!("restart: could not start the new process: {e}");
                    std::process::exit(RESTART_EXIT_CODE)
                }
            }
        }
    }
}

#[cfg(unix)]
fn exec(args: &[OsString]) -> std::io::Result<std::convert::Infallible> {
    use std::os::unix::process::CommandExt;
    // Only returns on failure.
    Err(std::process::Command::new(restart_exe()?).args(args).exec())
}

#[cfg(not(unix))]
fn exec(_: &[OsString]) -> std::io::Result<std::convert::Infallible> {
    Err(std::io::Error::other(
        "exec is not available on this platform",
    ))
}

/// Whether this platform can `exec`.
pub const HAS_EXEC: bool = cfg!(unix);

/// Waits for SIGINT or SIGTERM (Ctrl+C only on Windows).
pub async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => tracing::info!("SIGTERM"),
                    _ = int.recv() => tracing::info!("SIGINT"),
                }
            }
            _ => std::future::pending().await,
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C");
        } else {
            std::future::pending::<()>().await;
        }
    }
}

/// At the start of a restarted process on a platform without `exec`: gives the parent time to
/// release its ports.
pub async fn wait_for_parent() {
    if let Some(ms) = std::env::var(RESTART_WAIT_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        tokio::time::sleep(Duration::from_millis(ms.min(10_000))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_always_exits_zero() {
        for mode in [RestartMode::Exec, RestartMode::Exit] {
            for exec in [true, false] {
                assert_eq!(decide(ShutdownKind::Stop, mode, exec), Action::Exit(0));
            }
        }
    }

    #[test]
    fn a_restart_follows_the_mode_and_the_platform() {
        assert_eq!(
            decide(ShutdownKind::Restart, RestartMode::Exec, true),
            Action::Exec
        );
        assert_eq!(
            decide(ShutdownKind::Restart, RestartMode::Exec, false),
            Action::Spawn
        );
        for exec in [true, false] {
            assert_eq!(
                decide(ShutdownKind::Restart, RestartMode::Exit, exec),
                Action::Exit(75)
            );
        }
        assert_eq!(RESTART_EXIT_CODE, 75);
    }

    #[test]
    fn the_restart_path_is_the_running_binary() {
        let exe = restart_exe().unwrap();
        assert!(exe.exists(), "{}", exe.display());
    }
}
