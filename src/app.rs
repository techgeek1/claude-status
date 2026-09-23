use std::time::Duration;

use cosmic::{
    Element, Task,
    app,
    applet::padded_control,
    cosmic_theme::Spacing,
    iced::{
        self, Alignment, Border, Color, Length, Subscription, window,
        platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup},
        widget::{container, canvas, Stack},
    },
    theme,
    widget::{self, column, icon, row, text},
};

use crate::api::{self, StatusSummary, UsageResponse};
use crate::rc::{self, RcSession};
use crate::shared::{self, Feed, Refresh};

const APP_ID: &str = "dev.techgeek1.CosmicExtAppletClaudeStatus";
const STATUS_POLL_SECS: u64 = 300;
const POPUP_REFRESH_SECS: u64 = 60;
/// Minimum age of the shared cache before an open or refresh tick refetches.
const POPUP_TTL: Duration = Duration::from_secs(60);
/// Slightly under the poll interval, so of the N per-output instances ticking
/// at different phases, only the first each round actually fetches.
const STATUS_POLL_TTL: Duration = Duration::from_secs(STATUS_POLL_SECS - 10);
const POPUP_WIDTH: f32 = 340.0;
const BAR_GIRTH: f32 = 16.0;
const RC_INDICATOR_COLOR: Color = Color::from_rgb(0.25, 0.55, 0.95);

pub struct App {
    core: cosmic::app::Core,
    popup: Option<window::Id>,
    icon_handle: cosmic::widget::icon::Handle,
    usage: Option<UsageResponse>,
    status: Option<StatusSummary>,
    usage_meta: shared::Entry,
    status_meta: shared::Entry,
    fetching_usage: bool,
    fetching_status: bool,
    rc_sessions: Vec<RcSession>,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup,
    PopupClosed(window::Id),
    StatusTick,
    PopupRefreshTick,
    Refreshed(Feed, Result<Refresh, String>),
    /// Some instance (possibly this one) rewrote the shared cache.
    CacheChanged,
    RcTick,
}

impl App {
    fn fire_usage_fetch(&mut self, ttl: Duration) -> app::Task<Message> {
        if self.fetching_usage {
            return Task::none();
        }
        self.fetching_usage = true;
        cosmic::task::future(async move {
            let result = shared::refresh(Feed::Usage, ttl, api::fetch_usage).await;
            Message::Refreshed(Feed::Usage, result)
        })
    }

    fn fire_status_fetch(&mut self, ttl: Duration) -> app::Task<Message> {
        if self.fetching_status {
            return Task::none();
        }
        self.fetching_status = true;
        cosmic::task::future(async move {
            let result = shared::refresh(Feed::Status, ttl, api::fetch_status).await;
            Message::Refreshed(Feed::Status, result)
        })
    }

    fn apply_entry(&mut self, feed: Feed, entry: shared::Entry) {
        match feed {
            Feed::Usage => {
                self.usage = entry.parse();
                self.usage_meta = entry;
            }
            Feed::Status => {
                self.status = entry.parse();
                self.status_meta = entry;
            }
        }
    }

    fn reload_cache(&mut self) {
        self.apply_entry(Feed::Usage, shared::load(Feed::Usage));
        self.apply_entry(Feed::Status, shared::load(Feed::Status));
    }

    fn status_severity(&self) -> u8 {
        self.status
            .as_ref()
            .map(|s| api::status_severity(&s.status.indicator))
            .unwrap_or(0)
    }

}

impl cosmic::Application for App {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn init(core: cosmic::app::Core, _flags: ()) -> (Self, app::Task<Message>) {
        let icon_bytes: &[u8] =
            include_bytes!("../data/icons/hicolor/scalable/apps/claude-status.png");
        let icon_handle = cosmic::widget::icon::from_raster_bytes(icon_bytes);

        let mut app = App {
            core,
            popup: None,
            icon_handle,
            usage: None,
            status: None,
            usage_meta: shared::Entry::default(),
            status_meta: shared::Entry::default(),
            fetching_usage: false,
            fetching_status: false,
            rc_sessions: rc::scan_active(),
        };

        // Show whatever a sibling instance already fetched before asking.
        app.reload_cache();
        let status_task = app.fire_status_fetch(STATUS_POLL_TTL);
        (app, status_task)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    return destroy_popup(p);
                }

                let new_id = window::Id::unique();
                self.popup.replace(new_id);
                let popup_settings = self.core.applet.get_popup_settings(
                    self.core.main_window_id().unwrap(),
                    new_id,
                    None,
                    None,
                    None,
                );

                let usage_task = self.fire_usage_fetch(POPUP_TTL);
                let status_task = self.fire_status_fetch(POPUP_TTL);
                return Task::batch(vec![get_popup(popup_settings), usage_task, status_task]);
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::StatusTick => {
                return self.fire_status_fetch(STATUS_POLL_TTL);
            }
            Message::PopupRefreshTick => {
                if self.popup.is_some() {
                    let usage = self.fire_usage_fetch(POPUP_TTL);
                    let status = self.fire_status_fetch(POPUP_TTL);
                    return Task::batch(vec![usage, status]);
                }
            }
            Message::Refreshed(feed, result) => {
                match feed {
                    Feed::Usage => self.fetching_usage = false,
                    Feed::Status => self.fetching_status = false,
                }
                match result {
                    Ok(Refresh::Done(entry)) => self.apply_entry(feed, entry),
                    // The fetching instance's write reaches us as CacheChanged.
                    Ok(Refresh::Busy) => {}
                    Err(e) => tracing::warn!("{feed:?} cache refresh failed: {e}"),
                }
            }
            Message::CacheChanged => {
                self.reload_cache();
            }
            Message::RcTick => {
                self.rc_sessions = rc::scan_active();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let suggested = self.core.applet.suggested_size(false);
        let icon_size = suggested.0.min(suggested.1) as f32;
        let dot_size: f32 = (icon_size * 0.3).max(6.0);

        let sev = self.status_severity();
        let rc_attached = !self.rc_sessions.is_empty();

        let icon_widget = icon::icon(self.icon_handle.clone())
            .width(Length::Fixed(icon_size))
            .height(Length::Fixed(icon_size));

        let content: Element<'_, Message> = if sev > 0 || rc_attached {
            let mut stack = Stack::new()
                .push(icon_widget)
                .width(Length::Fixed(icon_size))
                .height(Length::Fixed(icon_size));

            if sev > 0 {
                stack = stack.push(overlay_dot(
                    severity_color(sev),
                    dot_size,
                    icon_size,
                    Alignment::End,
                    Alignment::End,
                ));
            }

            if rc_attached {
                stack = stack.push(overlay_dot(
                    RC_INDICATOR_COLOR,
                    dot_size,
                    icon_size,
                    Alignment::Start,
                    Alignment::End,
                ));
            }

            stack.into()
        } else {
            icon_widget.into()
        };

        self.core
            .applet
            .button_from_element(content, false)
            .on_press_down(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let Spacing {
            space_xxs,
            space_xs,
            space_s,
            ..
        } = theme::active().cosmic().spacing;

        let mut content = column![].spacing(space_xxs).width(Length::Fixed(POPUP_WIDTH));

        // --- Usage header with icon ---
        content = content.push(padded_control(
            row![
                text::heading("Usage"),
                widget::Space::new().width(Length::Fill),
                icon::icon(self.icon_handle.clone())
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0)),
            ]
            .spacing(space_xs)
            .align_y(Alignment::Center),
        ));

        if let Some(usage) = &self.usage {
            if usage.limits.is_empty() {
                // Fallback for a response without the generic `limits` list.
                let legacy = [
                    ("5-hour", &usage.five_hour),
                    ("7-day", &usage.seven_day),
                    ("Opus", &usage.seven_day_opus),
                    ("Sonnet", &usage.seven_day_sonnet),
                ];
                for (label, window) in legacy {
                    if let Some(w) = window {
                        content = content.push(usage_bar(
                            label,
                            w.utilization,
                            w.resets_at.as_deref(),
                            bar_color(w.utilization, None),
                            space_xs,
                        ));
                    }
                }
            } else {
                for limit in &usage.limits {
                    content = content.push(usage_bar(
                        limit_label(limit),
                        limit.percent,
                        limit.resets_at.as_deref(),
                        bar_color(limit.percent, limit.severity.as_deref()),
                        space_xs,
                    ));
                }
            }

            // Extra usage (pay-as-you-go overages). `spend` is the newer shape
            // and carries its own currency exponent; `extra_usage` is the older
            // cents-denominated one.
            if let Some(spend) = usage.spend.as_ref().filter(|s| s.enabled) {
                content = content.push(
                    padded_control(widget::divider::horizontal::default())
                        .padding([space_xxs, space_s]),
                );
                content = content.push(section_header("Extra Usage"));

                if let Some(used) = &spend.used {
                    let amount = match &spend.limit {
                        Some(limit) => format!(
                            "{:.2} / {:.2} {}",
                            used.amount(),
                            limit.amount(),
                            used.currency()
                        ),
                        None => format!("{:.2} {}", used.amount(), used.currency()),
                    };
                    content = content.push(padded_control(
                        row![
                            text::body("Spend"),
                            widget::Space::new().width(Length::Fill),
                            text::body(amount),
                        ]
                        .align_y(Alignment::Center),
                    ));
                }
                if let Some(percent) = spend.percent {
                    content = content.push(usage_bar(
                        "Budget",
                        percent,
                        None,
                        bar_color(percent, None),
                        space_xs,
                    ));
                }
            } else if let Some(extra) = usage.extra_usage.as_ref().filter(|e| e.is_enabled) {
                content = content.push(
                    padded_control(widget::divider::horizontal::default())
                        .padding([space_xxs, space_s]),
                );
                content = content.push(section_header("Extra Usage"));

                let currency = extra.currency.as_deref().unwrap_or("USD");
                if let (Some(used), Some(limit)) = (extra.used_credits, extra.monthly_limit) {
                    content = content.push(padded_control(
                        row![
                            text::body("Spend"),
                            widget::Space::new().width(Length::Fill),
                            text::body(format!(
                                "{:.2} / {:.2} {currency}",
                                used / 100.0,
                                limit / 100.0,
                            )),
                        ]
                        .align_y(Alignment::Center),
                    ));
                }
                if let Some(util) = extra.utilization {
                    content = content.push(usage_bar(
                        "Budget",
                        util,
                        None,
                        bar_color(util, None),
                        space_xs,
                    ));
                }
            }
        } else if self.usage_meta.error.is_none() {
            content = content.push(padded_control(text::body("Loading...")));
        }

        if let Some(caption) = freshness_caption(&self.usage_meta) {
            content = content.push(padded_control(text::caption(caption)));
        }

        // --- Remote control section ---
        //
        // Claude Code writes `bridgeSessionId` into ~/.claude/sessions/<pid>.json
        // while a remote-control client is attached.
        content = content.push(
            padded_control(widget::divider::horizontal::default())
                .padding([space_xxs, space_s]),
        );
        content = content.push(section_header("Remote Control"));

        let attached = self.rc_sessions.len();
        content = content.push(padded_control(
            row![
                rc_dot(attached > 0),
                text::body(match attached {
                    0 => "No sessions attached".to_string(),
                    1 => "1 session attached".to_string(),
                    n => format!("{n} sessions attached"),
                }),
            ]
            .spacing(space_xs)
            .align_y(Alignment::Center),
        ));

        for session in &self.rc_sessions {
            content = content.push(padded_control(text::caption(session.label())));
        }

        // --- Divider ---
        content = content.push(
            padded_control(widget::divider::horizontal::default()).padding([space_xxs, space_s]),
        );

        // --- Status section ---
        content = content.push(section_header("Service Status"));

        if let Some(status) = &self.status {
            // Overall status
            let sev = api::status_severity(&status.status.indicator);
            content = content.push(padded_control(
                row![
                    status_dot(sev),
                    text::body(&status.status.description),
                ]
                .spacing(space_xs)
                .align_y(Alignment::Center),
            ));

            // Per-component
            for component in &status.components {
                let comp_sev = api::status_severity(&component.status);
                content = content.push(padded_control(
                    row![
                        status_dot(comp_sev),
                        text::caption(&component.name),
                        widget::Space::new().width(Length::Fill),
                        text::caption(format_status(&component.status)),
                    ]
                    .spacing(space_xs)
                    .align_y(Alignment::Center),
                ));
            }

            // Active incidents
            if !status.incidents.is_empty() {
                content = content.push(
                    padded_control(widget::divider::horizontal::default())
                        .padding([space_xxs, space_s]),
                );
                content = content.push(section_header("Active Incidents"));

                for incident in &status.incidents {
                    let inc_sev = match incident.impact.as_str() {
                        "critical" => 3,
                        "major" => 2,
                        _ => 1,
                    };
                    content = content.push(padded_control(
                        row![
                            status_dot(inc_sev),
                            text::body(&incident.name),
                        ]
                        .spacing(space_xs)
                        .align_y(Alignment::Center),
                    ));

                    if let Some(update) = incident.incident_updates.first() {
                        content = content.push(padded_control(
                            text::caption(truncate(&update.body, 120)),
                        ));
                    }
                }
            }
        } else if self.status_meta.error.is_none() {
            content = content.push(padded_control(text::body("Loading...")));
        }

        if self.status.is_none()
            && let Some(caption) = freshness_caption(&self.status_meta)
        {
            content = content.push(padded_control(text::caption(caption)));
        }

        content = content.padding([8, 0]);

        self.core
            .applet
            .popup_container(container(content))
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::time::every(Duration::from_secs(STATUS_POLL_SECS)).map(|_| Message::StatusTick),
        ];

        subs.push(Subscription::run(rc_watch_stream));
        subs.push(Subscription::run(cache_watch_stream));

        if self.popup.is_some() {
            subs.push(
                iced::time::every(Duration::from_secs(POPUP_REFRESH_SECS))
                    .map(|_| Message::PopupRefreshTick),
            );
        }

        Subscription::batch(subs)
    }
}

fn rc_watch_stream() -> impl iced::futures::Stream<Item = Message> + Send {
    use iced::futures::StreamExt;
    rc::watch_events().map(|_| Message::RcTick)
}

fn cache_watch_stream() -> impl iced::futures::Stream<Item = Message> + Send {
    use iced::futures::StreamExt;
    shared::watch_events().map(|_| Message::CacheChanged)
}

// --- UI helpers ---

/// "Updated 14:32", plus the last error and next retry while backing off.
fn freshness_caption(meta: &shared::Entry) -> Option<String> {
    let clock = |ts: i64| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
    };
    let updated = meta.ok_at.and_then(clock).map(|t| format!("Updated {t}"));
    let failure = meta.error.as_deref().map(|err| {
        let err = truncate(err, 60);
        match meta.retry_at().and_then(clock) {
            Some(t) => format!("{err} \u{b7} retrying after {t}"),
            None => err,
        }
    });
    match (updated, failure) {
        (Some(u), Some(f)) => Some(format!("{u} \u{b7} {f}")),
        (u, f) => u.or(f),
    }
}

fn section_header(label: &str) -> Element<'_, Message> {
    padded_control(text::heading(label)).into()
}

/// Human-readable label for a limit entry. `kind` gives the window and the
/// optional `scope` names what it applies to, e.g. `weekly_scoped` scoped to
/// model "Fable" renders as "Weekly \u{b7} Fable".
fn limit_label(limit: &api::Limit) -> String {
    let base = match limit.kind.as_str() {
        "session" => "Session".to_string(),
        "weekly_all" | "weekly_scoped" => "Weekly".to_string(),
        other => humanize(other),
    };

    let qualifier = limit.scope.as_ref().and_then(|scope| {
        let names: Vec<&str> = [scope.model.as_ref(), scope.surface.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|entity| entity.display_name.as_deref())
            .collect();
        (!names.is_empty()).then(|| names.join(" / "))
    });

    match qualifier {
        Some(q) => format!("{base} \u{b7} {q}"),
        None => base,
    }
}

/// "weekly_all" -> "Weekly All". Only reached for limit kinds we don't know
/// about yet, so it just has to be readable, not pretty.
fn humanize(kind: &str) -> String {
    let mut out = String::with_capacity(kind.len());
    for (i, word) in kind.split('_').filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// Bar color from our own thresholds, escalated (never de-escalated) by the
/// server's severity hint, so an unfamiliar severity string is harmless.
fn bar_color(utilization: f64, severity: Option<&str>) -> BarColor {
    let by_threshold = if utilization >= 90.0 {
        2
    } else if utilization >= 70.0 {
        1
    } else {
        0
    };
    let by_severity = match severity {
        Some("critical" | "danger" | "exhausted" | "over_limit") => 2,
        Some("warning" | "warn" | "elevated") => 1,
        _ => 0,
    };

    match by_threshold.max(by_severity) {
        0 => BarColor::Success,
        1 => BarColor::Warning,
        _ => BarColor::Danger,
    }
}

fn usage_bar(
    label: impl Into<String>,
    utilization: f64,
    resets_at: Option<&str>,
    color: BarColor,
    spacing: u16,
) -> Element<'static, Message> {
    let reset_text = resets_at.and_then(format_reset_time).unwrap_or_default();

    let bar = canvas::Canvas::new(ProgressBarCanvas {
        progress: (utilization / 100.0).clamp(0.0, 1.0) as f32,
        color,
    })
    .width(Length::Fill)
    .height(Length::Fixed(BAR_GIRTH));

    let mut col = column![
        row![
            text::body(label.into()),
            widget::Space::new().width(Length::Fill),
            text::caption(format!("{:.0}%", utilization)),
        ]
        .align_y(Alignment::Center),
        bar,
    ]
    .spacing(spacing);

    if !reset_text.is_empty() {
        col = col.push(text::caption(reset_text));
    }

    padded_control(col).into()
}

// --- Canvas progress bar ---

#[derive(Clone, Copy)]
enum BarColor {
    Success,
    Warning,
    Danger,
}

struct ProgressBarCanvas {
    progress: f32,
    color: BarColor,
}

impl<Message> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for ProgressBarCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &cosmic::Renderer,
        theme: &cosmic::Theme,
        bounds: cosmic::iced::Rectangle,
        _cursor: cosmic::iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry<cosmic::Renderer>> {
        let cosmic_theme = theme.cosmic();
        let track_color = Color::from(cosmic_theme.background.divider);
        let bar_color = match self.color {
            BarColor::Success => Color::from(cosmic_theme.success.base),
            BarColor::Warning => Color::from(cosmic_theme.warning.base),
            BarColor::Danger => Color::from(cosmic_theme.destructive.base),
        };

        let w = bounds.width;
        let h = bounds.height;
        let radius = h / 2.0;

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Draw the rounded track
        let track = canvas::Path::rounded_rectangle(
            cosmic::iced::Point::ORIGIN,
            bounds.size(),
            radius.into(),
        );
        frame.fill(&track, track_color);

        // Draw the fill as a capsule. Earlier attempts to draw a rounded-left,
        // flat-right shape (composite arc path, circle + rectangle) rendered
        // with a squared-off left edge on the iced canvas backend, so we
        // settle for matching the track's capsule geometry end-to-end.
        //
        // For visibly non-zero progress, clamp the drawn width to at least
        // one bar-height so the curvature is actually visible — a sliver
        // narrower than the corner radius reads as a square pixel blob.
        let fill_width = (self.progress * w).min(w);
        if fill_width > 0.5 {
            let visible_width = fill_width.max(h).min(w);
            let fill = canvas::Path::rounded_rectangle(
                cosmic::iced::Point::ORIGIN,
                cosmic::iced::Size::new(visible_width, h),
                radius.into(),
            );
            frame.fill(&fill, bar_color);
        }

        vec![frame.into_geometry()]
    }
}

fn overlay_dot(
    color: Color,
    dot_size: f32,
    icon_size: f32,
    align_x: Alignment,
    align_y: Alignment,
) -> Element<'static, Message> {
    let dot = container(widget::Space::new().width(dot_size).height(dot_size)).class(
        cosmic::theme::Container::custom(move |_| cosmic::iced::widget::container::Style {
            background: Some(color.into()),
            border: Border::default().rounded(dot_size / 2.0),
            ..Default::default()
        }),
    );
    container(dot)
        .width(Length::Fixed(icon_size))
        .height(Length::Fixed(icon_size))
        .align_x(align_x)
        .align_y(align_y)
        .into()
}

fn severity_color(severity: u8) -> Color {
    match severity {
        0 => Color::from_rgb(0.2, 0.8, 0.2),
        1 => Color::from_rgb(0.9, 0.8, 0.1),
        2 => Color::from_rgb(0.9, 0.5, 0.1),
        _ => Color::from_rgb(0.9, 0.2, 0.2),
    }
}

fn status_dot(severity: u8) -> Element<'static, Message> {
    let color = severity_color(severity);

    container(widget::Space::new().width(8).height(8))
        .class(cosmic::theme::Container::custom(move |_theme| {
            cosmic::iced::widget::container::Style {
                background: Some(color.into()),
                border: Border::default().rounded(4),
                ..Default::default()
            }
        }))
        .into()
}

fn rc_dot(active: bool) -> Element<'static, Message> {
    let color = if active {
        RC_INDICATOR_COLOR
    } else {
        Color::from_rgba(0.5, 0.5, 0.5, 0.4)
    };
    container(widget::Space::new().width(8).height(8))
        .class(cosmic::theme::Container::custom(move |_theme| {
            cosmic::iced::widget::container::Style {
                background: Some(color.into()),
                border: Border::default().rounded(4),
                ..Default::default()
            }
        }))
        .into()
}

fn format_status(status: &str) -> &str {
    match status {
        "operational" => "OK",
        "degraded_performance" => "Degraded",
        "partial_outage" => "Partial Outage",
        "major_outage" => "Major Outage",
        "under_maintenance" => "Maintenance",
        _ => status,
    }
}

fn format_reset_time(iso: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let now = chrono::Utc::now();
    let diff = dt.signed_duration_since(now);

    if diff.num_seconds() <= 0 {
        return Some("Resetting...".into());
    }

    let total_secs = diff.num_seconds();
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;

    if days > 0 {
        Some(format!("Resets in {days}d {hours}h"))
    } else if hours > 0 {
        Some(format!("Resets in {hours}h {minutes}m"))
    } else {
        Some(format!("Resets in {minutes}m"))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
