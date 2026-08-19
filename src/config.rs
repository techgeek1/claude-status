use cosmic::cosmic_config::{self, ConfigGet, ConfigSet};

/// Bumped only on a breaking change to the stored key set.
const CONFIG_VERSION: u64 = 1;
const AUTO_INHIBIT_KEY: &str = "auto_inhibit_remote";

/// Applet settings, backed by cosmic-config.
///
/// A missing or unreadable config is not an error: we fall back to the
/// in-memory defaults and keep running, so a broken XDG config dir costs the
/// user a preference rather than the applet.
pub struct Settings {
    handle: Option<cosmic_config::Config>,
    auto_inhibit_remote: bool,
}

impl Settings {
    pub fn load(app_id: &str) -> Self {
        let handle = match cosmic_config::Config::new(app_id, CONFIG_VERSION) {
            Ok(h) => Some(h),
            Err(e) => {
                tracing::warn!("config unavailable, using defaults: {e}");
                None
            }
        };
        let auto_inhibit_remote = handle
            .as_ref()
            .and_then(|h| h.get::<bool>(AUTO_INHIBIT_KEY).ok())
            .unwrap_or(true);

        Self {
            handle,
            auto_inhibit_remote,
        }
    }

    pub fn auto_inhibit_remote(&self) -> bool {
        self.auto_inhibit_remote
    }

    pub fn set_auto_inhibit_remote(&mut self, enabled: bool) {
        self.auto_inhibit_remote = enabled;
        if let Some(handle) = &self.handle
            && let Err(e) = handle.set(AUTO_INHIBIT_KEY, enabled)
        {
            tracing::warn!("failed to persist {AUTO_INHIBIT_KEY}: {e}");
        }
    }
}
