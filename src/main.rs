mod app;
mod api;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    cosmic::applet::run::<app::App>(())
}
