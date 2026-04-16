use zbus::{Connection, proxy, zvariant::OwnedFd};

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
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
        .inhibit(
            "sleep:idle",
            "claude-status",
            "Claude Code remote-control session active",
            "block",
        )
        .await
        .map_err(|e| format!("Inhibit: {e}"))?;
    Ok(InhibitLock { _fd: fd })
}
