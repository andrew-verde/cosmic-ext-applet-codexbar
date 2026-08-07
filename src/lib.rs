mod codexbar;
mod window;

pub use codexbar::{ProviderPayload, parse_usage_json};

use window::Window;

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Window>(())
}
