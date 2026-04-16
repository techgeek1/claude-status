use std::path::PathBuf;
use std::time::Duration;

use futures::{Stream, StreamExt};
use inotify::{Inotify, WatchMask};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct RcSession {
    pub cwd: String,
    pub name: Option<String>,
}

impl RcSession {
    pub fn label(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        std::path::Path::new(&self.cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.cwd.clone())
    }
}

#[derive(Deserialize)]
struct SessionFile {
    pid: i32,
    cwd: String,
    name: Option<String>,
    #[serde(rename = "bridgeSessionId", default)]
    bridge_session_id: Option<String>,
}

fn sessions_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("sessions"))
}

fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let Some(p) = rustix::process::Pid::from_raw(pid) else {
        return false;
    };
    rustix::process::test_kill_process(p).is_ok()
}

const FALLBACK_POLL: Duration = Duration::from_secs(10);

/// Stream that yields `()` whenever `~/.claude/sessions/` changes.
///
/// Emits once immediately for initial scan. If inotify setup fails (or the
/// sessions directory doesn't exist yet), falls back to polling every 10s
/// until the watch can be established.
pub fn watch_events() -> impl Stream<Item = ()> + Send {
    async_stream::stream! {
        yield ();
        loop {
            match open_watch() {
                Ok(mut events) => {
                    while let Some(evt) = events.next().await {
                        match evt {
                            Ok(_) => yield (),
                            Err(e) => {
                                tracing::warn!("rc inotify read error: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("rc inotify setup failed: {e}; polling");
                }
            }
            tokio::time::sleep(FALLBACK_POLL).await;
            yield ();
        }
    }
}

fn open_watch() -> std::io::Result<inotify::EventStream<Vec<u8>>> {
    let dir = sessions_dir()
        .ok_or_else(|| std::io::Error::other("no home directory"))?;
    let inotify = Inotify::init()?;
    inotify.watches().add(
        &dir,
        WatchMask::CREATE
            | WatchMask::MODIFY
            | WatchMask::CLOSE_WRITE
            | WatchMask::DELETE
            | WatchMask::MOVED_FROM
            | WatchMask::MOVED_TO,
    )?;
    inotify.into_event_stream(vec![0u8; 1024])
}

pub fn scan_active() -> Vec<RcSession> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(sf) = serde_json::from_slice::<SessionFile>(&bytes) else {
            continue;
        };
        if sf
            .bridge_session_id
            .as_deref()
            .map_or(true, str::is_empty)
        {
            continue;
        }
        if !pid_alive(sf.pid) {
            continue;
        }
        out.push(RcSession {
            cwd: sf.cwd,
            name: sf.name,
        });
    }
    out
}
