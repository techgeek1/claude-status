use std::process::Command;

use zbus::{Connection, proxy, zvariant::OwnedFd};

const WHO: &str = "claude-status";

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(name = "ListInhibitors")]
    fn list_inhibitors(
        &self,
    ) -> zbus::Result<Vec<(String, String, String, String, u32, u32)>>;
}

#[derive(Debug)]
pub struct InhibitLock {
    _fd: OwnedFd,
}

pub async fn acquire() -> Result<InhibitLock, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("connect system bus: {e}"))?;
    let proxy = LoginManagerProxy::new(&conn)
        .await
        .map_err(|e| format!("login1 proxy: {e}"))?;
    let fd = proxy
        .inhibit("sleep:idle", WHO, "User-requested sleep inhibit", "block")
        .await
        .map_err(|e| format!("Inhibit: {e}"))?;
    Ok(InhibitLock { _fd: fd })
}

/// SIGTERM any orphaned `claude-status` inhibitor holders left over from a
/// previous applet instance. logind keeps an inhibitor alive as long as the
/// owning fd is open, so a stale applet process holds the lock invisibly —
/// the new applet has no fd handle to it and can't release it. Killing the
/// owning PID closes the fd and lets logind drop the lock.
///
/// Returns the number of PIDs we sent SIGTERM to.
pub async fn cleanup_orphans() -> Result<usize, String> {
    let our_pid = std::process::id();
    let conn = Connection::system()
        .await
        .map_err(|e| format!("connect system bus: {e}"))?;
    let proxy = LoginManagerProxy::new(&conn)
        .await
        .map_err(|e| format!("login1 proxy: {e}"))?;
    let inhibitors = proxy
        .list_inhibitors()
        .await
        .map_err(|e| format!("ListInhibitors: {e}"))?;

    let mut killed = 0usize;
    for (what, who, _why, _mode, _uid, pid) in inhibitors {
        if who != WHO || pid == our_pid {
            continue;
        }
        tracing::warn!(
            "found orphaned inhibitor (pid={pid}, what={what}); sending SIGTERM"
        );
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        killed += 1;
    }
    Ok(killed)
}
