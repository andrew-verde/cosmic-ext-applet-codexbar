mod codexbar;
mod config;
mod window;

pub use codexbar::{CostPayload, ProviderPayload, parse_cost_json, parse_usage_json};
pub use config::{Config, UsageDisplay};

use window::Window;

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Window>(())
}
