use std::time::Duration;

use chrono::Utc;
use cosmic::app::Core;
use cosmic::applet::padded_control;
use cosmic::iced::{
    Length, Limits, Subscription,
    platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup},
    time,
    window::Id,
};
use cosmic::widget;
use cosmic::{Action, Application, Element, Task};

use crate::codexbar::{ProviderPayload, fetch_usage};

const ID: &str = "dev.andrewgreen.codexbar";
const ICON: &str = "dev.andrewgreen.codexbar-symbolic";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    Refresh,
    UsageFetched(Result<Vec<ProviderPayload>, String>),
}

enum State {
    Loading,
    Loaded(Vec<ProviderPayload>),
    Failed(String),
}

pub struct Window {
    core: Core,
    popup: Option<Id>,
    state: State,
}

impl Application for Window {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Action<Message>>) {
        let window = Window {
            core,
            popup: None,
            state: State::Loading,
        };
        (window, refresh_task())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(REFRESH_INTERVAL).map(|_| Message::Refresh)
    }

    fn update(&mut self, message: Message) -> Task<Action<Message>> {
        match message {
            Message::TogglePopup => {
                if let Some(popup) = self.popup.take() {
                    return destroy_popup(popup);
                }
                let new_id = Id::unique();
                self.popup.replace(new_id);
                let mut popup_settings = self.core.applet.get_popup_settings(
                    self.core.main_window_id().unwrap(),
                    new_id,
                    None,
                    None,
                    None,
                );
                popup_settings.positioner.size_limits = popup_limits();
                return Task::batch([refresh_task(), get_popup(popup_settings)]);
            }
            Message::PopupClosed(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::Refresh => return refresh_task(),
            Message::UsageFetched(Ok(payloads)) => self.state = State::Loaded(payloads),
            Message::UsageFetched(Err(error)) => self.state = State::Failed(error),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button(ICON)
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let content = match &self.state {
            State::Loading => widget::Column::new().push(widget::text::body("Loading usage…")),
            State::Failed(error) => widget::Column::new()
                .spacing(4)
                .push(widget::text::title4("CodexBar unavailable"))
                .push(widget::text::body(error.clone())),
            State::Loaded(payloads) if payloads.is_empty() => widget::Column::new()
                .spacing(4)
                .push(widget::text::title4("No providers"))
                .push(widget::text::body(
                    "Enable one with `codexbar config enable --provider <id>`.",
                )),
            State::Loaded(payloads) => {
                let mut column = widget::Column::new().spacing(12);
                for payload in payloads {
                    column = column.push(provider_view(payload));
                }
                column
            }
        };

        self.core
            .applet
            .popup_container(padded_control(content.width(Length::Fill)))
            .limits(popup_limits())
            .into()
    }
}

fn popup_limits() -> Limits {
    Limits::NONE
        .min_width(320.0)
        .max_width(420.0)
        .max_height(600.0)
}

fn refresh_task() -> Task<Action<Message>> {
    Task::perform(fetch_usage(), |result| {
        Message::UsageFetched(result).into()
    })
}

fn provider_view(payload: &ProviderPayload) -> Element<'_, Message> {
    let mut column = widget::Column::new().spacing(6).push(header(payload));

    if let Some(error) = &payload.error {
        column = column.push(widget::text::body(error.message.clone()));
        return column.into();
    }

    let Some(usage) = &payload.usage else {
        column = column.push(widget::text::body("No usage data reported."));
        return column.into();
    };

    let now = Utc::now();
    let windows = [
        (usage.primary.as_ref(), "Primary"),
        (usage.secondary.as_ref(), "Secondary"),
        (usage.tertiary.as_ref(), "Tertiary"),
    ];

    let mut any = false;
    for (window, fallback) in windows {
        let Some(window) = window else { continue };
        any = true;
        let used = window.used_percent.unwrap_or(0.0);
        let mut row = widget::Row::new()
            .spacing(8)
            .push(widget::text::body(window.window_label(fallback)).width(Length::Fixed(80.0)))
            .push(widget::text::body(format!("{used:.0}%")));
        if let Some(reset) = window.reset_text(now) {
            row = row.push(widget::text::caption(reset));
        }
        column = column
            .push(row)
            .push(widget::determinate_linear(window.fraction()));
    }

    if !any {
        column = column.push(widget::text::body("No limit windows reported."));
    }

    if let Some(remaining) = payload.credits.as_ref().and_then(|c| c.remaining) {
        column = column.push(widget::text::caption(format!("Credits: {remaining:.2}")));
    }

    column.into()
}

fn header(payload: &ProviderPayload) -> Element<'_, Message> {
    let mut row = widget::Row::new()
        .spacing(8)
        .push(widget::text::title4(payload.label()));
    if let Some(account) = &payload.account {
        row = row.push(widget::text::caption(account.clone()));
    }
    row.into()
}
