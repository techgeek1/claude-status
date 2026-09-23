//! Cross-process fetch cache.
//!
//! cosmic-panel runs one applet process per output, so without coordination
//! every monitor polls the APIs independently and burns through rate limits N
//! times as fast. All instances instead share one cache file per feed under
//! `$XDG_RUNTIME_DIR/claude-status/`. A non-blocking `flock` makes whichever
//! instance gets there first the fetcher; the rest skip the request and pick
//! the result up through an inotify watch on the cache directory. The kernel
//! drops the lock if the fetcher dies, so there is no leader state to recover.

use std::fs::File;
use std::path::PathBuf;
use std::time::Duration;

use futures::{Stream, StreamExt};
use inotify::{Inotify, WatchMask};
use rustix::fs::FlockOperation;
use rustix::io::Errno;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::api::FetchError;

const BACKOFF_BASE_SECS: i64 = 60;
const BACKOFF_MAX_SECS: i64 = 15 * 60;
const FALLBACK_POLL: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
pub enum Feed {
    Usage,
    Status,
}

impl Feed {
    fn name(self) -> &'static str {
        match self {
            Feed::Usage => "usage",
            Feed::Status => "status",
        }
    }
}

/// On-disk state for one feed. Timestamps are unix seconds, since they're
/// compared across processes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    /// Last successfully fetched response body. Kept across failures so a
    /// transient error doesn't blank the display.
    pub body: Option<serde_json::Value>,
    pub ok_at: Option<i64>,
    /// Last attempt, successful or not. This is what the TTL gates, so a
    /// failing endpoint isn't retried any faster than a healthy one.
    pub attempted_at: Option<i64>,
    pub error: Option<String>,
    #[serde(default)]
    pub failures: u32,
    pub backoff_until: Option<i64>,
}

impl Entry {
    pub fn parse<T: DeserializeOwned>(&self) -> Option<T> {
        let body = self.body.as_ref()?;
        match T::deserialize(body) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("cached body no longer parses: {e}");
                None
            }
        }
    }

    /// Unix seconds of the next allowed attempt, if we're backing off.
    pub fn retry_at(&self) -> Option<i64> {
        self.backoff_until.filter(|&t| t > now())
    }
}

#[derive(Debug, Clone)]
pub enum Refresh {
    Done(Entry),
    /// Another instance holds the lock and is fetching; its result arrives
    /// through [`watch_events`].
    Busy,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn dir() -> Option<PathBuf> {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .map(|d| d.join("claude-status"))
}

fn entry_path(feed: Feed) -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{}.json", feed.name())))
}

/// Read the cached entry for `feed`. Missing or corrupt files read as empty.
pub fn load(feed: Feed) -> Entry {
    entry_path(feed)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn store(feed: Feed, entry: &Entry) -> Result<(), String> {
    let path = entry_path(feed).ok_or("no cache directory")?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(entry).map_err(|e| format!("serialize cache: {e}"))?;
    // Write-then-rename so readers never see a torn file. Only the lock
    // holder writes, so the tmp name can't collide.
    std::fs::write(&tmp, bytes).map_err(|e| format!("write cache: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename cache: {e}"))
}

fn backoff_secs(failures: u32) -> i64 {
    let shift = failures.saturating_sub(1).min(16);
    (BACKOFF_BASE_SECS << shift).min(BACKOFF_MAX_SECS)
}

/// Fetch `feed` unless the shared cache is younger than `ttl`, we're inside a
/// backoff window, or another instance is already fetching.
pub async fn refresh<F, Fut>(feed: Feed, ttl: Duration, fetch: F) -> Result<Refresh, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<serde_json::Value, FetchError>>,
{
    let dir = dir().ok_or("no cache directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir: {e}"))?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{}.lock", feed.name())))
        .map_err(|e| format!("open lock: {e}"))?;
    match rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(Errno::WOULDBLOCK) => return Ok(Refresh::Busy),
        Err(e) => return Err(format!("flock: {e}")),
    }

    // Re-read under the lock: another instance may have just finished.
    let mut entry = load(feed);
    let now = now();
    let fresh = entry
        .attempted_at
        .is_some_and(|t| now - t < ttl.as_secs() as i64);
    if fresh || entry.retry_at().is_some() {
        return Ok(Refresh::Done(entry));
    }

    entry.attempted_at = Some(now);
    match fetch().await {
        Ok(body) => {
            entry.body = Some(body);
            entry.ok_at = Some(now);
            entry.error = None;
            entry.failures = 0;
            entry.backoff_until = None;
        }
        Err(e) => {
            entry.failures += 1;
            let hinted = e.retry_after.map_or(0, |d| d.as_secs() as i64);
            let wait = backoff_secs(entry.failures).max(hinted);
            tracing::warn!("{} fetch failed ({}x), backing off {wait}s: {}", feed.name(), entry.failures, e.message);
            entry.backoff_until = Some(now + wait);
            entry.error = Some(e.message);
        }
    }

    store(feed, &entry)?;
    Ok(Refresh::Done(entry))
}

/// Stream that yields `()` whenever any instance updates the cache.
///
/// Emits once immediately. Falls back to polling if the directory can't be
/// created or watched, retrying the watch each round.
pub fn watch_events() -> impl Stream<Item = ()> + Send {
    async_stream::stream! {
        yield ();
        loop {
            match open_watch() {
                Ok(mut events) => {
                    while let Some(evt) = events.next().await {
                        match evt {
                            Ok(e) if e.name.as_ref().is_some_and(|n| n.to_string_lossy().ends_with(".json")) => yield (),
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!("cache inotify read error: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("cache inotify setup failed: {e}; polling");
                }
            }
            tokio::time::sleep(FALLBACK_POLL).await;
            yield ();
        }
    }
}

fn open_watch() -> std::io::Result<inotify::EventStream<Vec<u8>>> {
    let dir = dir().ok_or_else(|| std::io::Error::other("no cache directory"))?;
    std::fs::create_dir_all(&dir)?;
    let inotify = Inotify::init()?;
    // `store` only ever renames into place.
    inotify.watches().add(&dir, WatchMask::MOVED_TO)?;
    inotify.into_event_stream(vec![0u8; 1024])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn done(r: Result<Refresh, String>) -> Entry {
        match r.unwrap() {
            Refresh::Done(e) => e,
            Refresh::Busy => panic!("unexpected Busy"),
        }
    }

    // One test, since it points XDG_RUNTIME_DIR at a scratch dir for the
    // whole process.
    #[test]
    fn ttl_lock_and_backoff() {
        let tmp = std::env::temp_dir().join(format!("claude-status-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &tmp) };
        let ttl = Duration::from_secs(60);

        // First call fetches; a second inside the TTL is served from cache.
        let e = done(run(refresh(Feed::Usage, ttl, || async { Ok(serde_json::json!({"n": 1})) })));
        assert_eq!(e.body, Some(serde_json::json!({"n": 1})));
        let e = done(run(refresh(Feed::Usage, ttl, || async { panic!("fetched inside TTL") })));
        assert_eq!(e.body, Some(serde_json::json!({"n": 1})));

        // While another holder has the lock, we don't fetch.
        let held = File::open(tmp.join("claude-status/usage.lock")).unwrap();
        rustix::fs::flock(&held, FlockOperation::LockExclusive).unwrap();
        let r = run(refresh(Feed::Usage, Duration::ZERO, || async { panic!("fetched while locked") }));
        assert!(matches!(r, Ok(Refresh::Busy)));
        drop(held);

        // A failure keeps the old body and honors a Retry-After above the base backoff.
        let e = done(run(refresh(Feed::Usage, Duration::ZERO, || async {
            Err(FetchError {
                message: "429".into(),
                retry_after: Some(Duration::from_secs(600)),
            })
        })));
        assert_eq!(e.body, Some(serde_json::json!({"n": 1})));
        assert_eq!(e.failures, 1);
        assert_eq!(e.backoff_until.unwrap() - e.attempted_at.unwrap(), 600);
        // Even a zero TTL doesn't fetch during backoff.
        let e = done(run(refresh(Feed::Usage, Duration::ZERO, || async { panic!("fetched in backoff") })));
        assert_eq!(e.error.as_deref(), Some("429"));

        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(3), 240);
        assert_eq!(backoff_secs(10), BACKOFF_MAX_SECS);

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
