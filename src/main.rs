mod api;
mod app;
mod config;
mod inhibit;
mod rc;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    cosmic::applet::run::<app::App>(())
}
