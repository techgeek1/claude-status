use std::time::{Duration, Instant};

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

const APP_ID: &str = "dev.techgeek1.CosmicExtAppletClaudeStatus";
const STATUS_POLL_SECS: u64 = 300;
const POPUP_REFRESH_SECS: u64 = 60;
const DEBOUNCE: Duration = Duration::from_secs(5);
const POPUP_WIDTH: f32 = 340.0;
const BAR_GIRTH: f32 = 16.0;

pub struct App {
    core: cosmic::app::Core,
    popup: Option<window::Id>,
    icon_handle: cosmic::widget::icon::Handle,
    usage: Option<UsageResponse>,
    status: Option<StatusSummary>,
    usage_error: Option<String>,
    status_error: Option<String>,
    fetching_usage: bool,
    fetching_status: bool,
    last_usage_fetch: Option<Instant>,
    last_status_fetch: Option<Instant>,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup,
    PopupClosed(window::Id),
    StatusTick,
    PopupRefreshTick,
    StatusResult(Result<StatusSummary, String>),
    UsageResult(Result<UsageResponse, String>),
}

impl App {
    fn should_fetch_usage(&self) -> bool {
        !self.fetching_usage
            && self
                .last_usage_fetch
                .map_or(true, |t| t.elapsed() >= DEBOUNCE)
    }

    fn should_fetch_status(&self) -> bool {
        !self.fetching_status
            && self
                .last_status_fetch
                .map_or(true, |t| t.elapsed() >= DEBOUNCE)
    }

    fn fire_usage_fetch(&mut self) -> app::Task<Message> {
        if !self.should_fetch_usage() {
            return Task::none();
        }
        self.fetching_usage = true;
        self.last_usage_fetch = Some(Instant::now());
        cosmic::task::future(async { Message::UsageResult(api::fetch_usage().await) })
    }

    fn fire_status_fetch(&mut self) -> app::Task<Message> {
        if !self.should_fetch_status() {
            return Task::none();
        }
        self.fetching_status = true;
        self.last_status_fetch = Some(Instant::now());
        cosmic::task::future(async { Message::StatusResult(api::fetch_status().await) })
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
            usage_error: None,
            status_error: None,
            fetching_usage: false,
            fetching_status: false,
            last_usage_fetch: None,
            last_status_fetch: None,
        };

        let task = app.fire_status_fetch();
        (app, task)
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

                let usage_task = self.fire_usage_fetch();
                let status_task = self.fire_status_fetch();
                return Task::batch(vec![get_popup(popup_settings), usage_task, status_task]);
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::StatusTick => {
                return self.fire_status_fetch();
            }
            Message::PopupRefreshTick => {
                if self.popup.is_some() {
                    let usage = self.fire_usage_fetch();
                    let status = self.fire_status_fetch();
                    return Task::batch(vec![usage, status]);
                }
            }
            Message::StatusResult(result) => {
                self.fetching_status = false;
                match result {
                    Ok(status) => {
                        self.status_error = None;
                        self.status = Some(status);
                    }
                    Err(e) => {
                        tracing::warn!("Status fetch failed: {e}");
                        self.status_error = Some(e);
                    }
                }
            }
            Message::UsageResult(result) => {
                self.fetching_usage = false;
                match result {
                    Ok(usage) => {
                        self.usage_error = None;
                        self.usage = Some(usage);
                    }
                    Err(e) => {
                        tracing::warn!("Usage fetch failed: {e}");
                        self.usage_error = Some(e);
                    }
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let suggested = self.core.applet.suggested_size(false);
        let icon_size = suggested.0.min(suggested.1) as f32;
        let dot_size: f32 = (icon_size * 0.3).max(6.0);

        let sev = self.status_severity();

        let icon_widget = icon::icon(self.icon_handle.clone())
            .width(Length::Fixed(icon_size))
            .height(Length::Fixed(icon_size));

        let content: Element<'_, Message> = if sev > 0 {
            let dot_color = severity_color(sev);
            let dot = container(widget::Space::new().width(dot_size).height(dot_size))
                .class(cosmic::theme::Container::custom(move |_| {
                    cosmic::iced::widget::container::Style {
                        background: Some(dot_color.into()),
                        border: Border::default().rounded(dot_size / 2.0),
                        ..Default::default()
                    }
                }));

            let dot_positioned = container(dot)
                .width(Length::Fixed(icon_size))
                .height(Length::Fixed(icon_size))
                .align_x(Alignment::End)
                .align_y(Alignment::End);

            Stack::new()
                .push(icon_widget)
                .push(dot_positioned)
                .width(Length::Fixed(icon_size))
                .height(Length::Fixed(icon_size))
                .into()
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
            if let Some(w) = &usage.five_hour {
                content = content.push(usage_bar("5-hour", w.utilization, &w.resets_at, space_xs));
            }
            if let Some(w) = &usage.seven_day {
                content = content.push(usage_bar("7-day", w.utilization, &w.resets_at, space_xs));
            }
            if let Some(w) = &usage.seven_day_sonnet {
                content = content.push(usage_bar("Sonnet", w.utilization, &w.resets_at, space_xs));
            }
            // Extra usage (pay-as-you-go overages)
            if let Some(extra) = &usage.extra_usage {
                if extra.is_enabled {
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
                                    "${:.2} / ${:.2} {currency}",
                                    used / 100.0,
                                    limit / 100.0,
                                )),
                            ]
                            .align_y(Alignment::Center),
                        ));
                    }
                    if let Some(util) = extra.utilization {
                        content = content.push(usage_bar("Budget", util, &None, space_xs));
                    }
                }
            }
        } else if let Some(err) = &self.usage_error {
            content = content.push(padded_control(text::body(truncate(err, 60))));
        } else if self.fetching_usage {
            content = content.push(padded_control(text::body("Loading...")));
        } else {
            content = content.push(padded_control(text::body("Click to refresh")));
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
        } else if let Some(err) = &self.status_error {
            content = content.push(padded_control(text::body(truncate(err, 60))));
        } else {
            content = content.push(padded_control(text::body("Loading...")));
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

        if self.popup.is_some() {
            subs.push(
                iced::time::every(Duration::from_secs(POPUP_REFRESH_SECS))
                    .map(|_| Message::PopupRefreshTick),
            );
        }

        Subscription::batch(subs)
    }
}

// --- UI helpers ---

fn section_header(label: &str) -> Element<'_, Message> {
    padded_control(text::heading(label)).into()
}

fn usage_bar<'a>(
    label: &'a str,
    utilization: f64,
    resets_at: &'a Option<String>,
    spacing: u16,
) -> Element<'a, Message> {
    let reset_text = resets_at
        .as_deref()
        .and_then(format_reset_time)
        .unwrap_or_default();

    let bar_color = if utilization >= 90.0 {
        BarColor::Danger
    } else if utilization >= 70.0 {
        BarColor::Warning
    } else {
        BarColor::Success
    };

    let bar = canvas::Canvas::new(ProgressBarCanvas {
        progress: (utilization / 100.0).clamp(0.0, 1.0) as f32,
        color: bar_color,
    })
    .width(Length::Fill)
    .height(Length::Fixed(BAR_GIRTH));

    let mut col = column![
        row![
            text::body(label),
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
