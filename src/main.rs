mod api;
mod app;
mod rc;
mod shared;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    cosmic::applet::run::<app::App>(())
}
