use std::sync::LazyLock;
use std::time::Duration;

use chrono::Utc;
use cosmic::app::Core;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::padded_control;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Container;
use cosmic::iced::{
    Border, Color, Length, Limits, Shadow, Subscription,
    platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup},
    time,
    window::Id,
};
use cosmic::widget;
use cosmic::widget::autosize::{Autosize, autosize};
use cosmic::{Action, Application, Element, Renderer, Task};

use crate::codexbar::{ProviderPayload, fetch_usage};
use crate::config::Config;

const ID: &str = "dev.andrewgreen.codexbar";
const ICON: &str = "dev.andrewgreen.codexbar-symbolic";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Identifies the autosizing popup body to the shell, mirroring the private
/// `AUTOSIZE_ID` that `cosmic::applet::Context::popup_container` uses.
static AUTOSIZE_ID: LazyLock<cosmic::iced::id::Id> =
    LazyLock::new(|| cosmic::iced::id::Id::new("codexbar-applet-autosize"));

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
    config: Config,
    /// Why the config file on disk was not honoured, shown in the popup.
    config_error: Option<String>,
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
        let (config, config_error) = crate::config::load();
        let window = Window {
            core,
            popup: None,
            state: State::Loading,
            config,
            config_error,
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
                self.reload_config();
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
            Message::Refresh => {
                self.reload_config();
                return refresh_task();
            }
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
                let mut column = widget::Column::new().spacing(12).push(widget::text::caption(
                    format!("Showing: {}", self.config.usage_display.label()),
                ));
                for payload in payloads {
                    column = column.push(provider_view(payload, &self.config));
                }
                column
            }
        };

        let content = match &self.config_error {
            Some(error) => widget::Column::new()
                .spacing(8)
                .push(content)
                .push(widget::text::caption(error.clone())),
            None => content,
        };

        self.popup_container(padded_control(content.width(Length::Fill)))
            .limits(popup_limits())
            .into()
    }
}

impl Window {
    fn reload_config(&mut self) {
        let (config, config_error) = crate::config::load();
        self.config = config;
        self.config_error = config_error;
    }

    /// `cosmic::applet::Context::popup_container`, reproduced here so the
    /// background alpha can be scaled by `background_opacity`. At the default
    /// opacity of `1.0` this renders exactly what the upstream helper does.
    fn popup_container<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Autosize<'a, Message, cosmic::Theme, Renderer> {
        let (vertical_align, horizontal_align) = match self.core.applet.anchor {
            PanelAnchor::Left => (Vertical::Center, Horizontal::Left),
            PanelAnchor::Right => (Vertical::Center, Horizontal::Right),
            PanelAnchor::Top => (Vertical::Top, Horizontal::Center),
            PanelAnchor::Bottom => (Vertical::Bottom, Horizontal::Center),
        };
        let opacity = self.config.background_opacity;

        autosize(
            Container::<Message, _, Renderer>::new(
                Container::<Message, _, Renderer>::new(content).style(move |theme| {
                    let cosmic = theme.cosmic();
                    let background = cosmic.background(theme.transparent);
                    let mut bg = Color::from(background.base);
                    bg.a *= opacity;
                    cosmic::iced::widget::container::Style {
                        text_color: Some(background.on.into()),
                        background: Some(bg.into()),
                        border: Border {
                            radius: cosmic.corner_radii.radius_m.into(),
                            width: 1.0,
                            color: background.divider.into(),
                        },
                        shadow: Shadow::default(),
                        icon_color: Some(background.on.into()),
                        snap: true,
                    }
                }),
            )
            .height(Length::Shrink)
            .align_x(horizontal_align)
            .align_y(vertical_align),
            AUTOSIZE_ID.clone(),
        )
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

fn provider_view<'a>(payload: &'a ProviderPayload, config: &Config) -> Element<'a, Message> {
    let mut column = widget::Column::new()
        .spacing(6)
        .push(header(payload, config));

    if let Some(error) = &payload.error {
        column = column.push(widget::text::body(error.message.clone()));
        return column.into();
    }

    let Some(usage) = &payload.usage else {
        column = column.push(widget::text::body("No usage data reported."));
        return column.into();
    };

    let now = Utc::now();
    let pace = payload.pace.as_ref();
    let windows = [
        (
            usage.primary.as_ref(),
            "Primary",
            config.show_session,
            pace.and_then(|p| p.primary.as_ref()),
        ),
        (
            usage.secondary.as_ref(),
            "Secondary",
            config.show_weekly,
            pace.and_then(|p| p.secondary.as_ref()),
        ),
        (
            usage.tertiary.as_ref(),
            "Tertiary",
            config.show_monthly,
            pace.and_then(|p| p.tertiary.as_ref()),
        ),
    ];

    // `any` tracks whether the *data* is present, not whether it is displayed,
    // so hiding every window with the config still leaves the provider silent
    // rather than claiming nothing was reported.
    let mut any = false;
    for (window, fallback, show, pace) in windows {
        let Some(window) = window else { continue };
        any = true;
        if !show {
            continue;
        }
        let percent = config.usage_display.percent(window.used_percent.unwrap_or(0.0));
        // The label/percent row is kept short and every caption goes on its own
        // full-width line below it: crammed into the row, long strings such as
        // "Resets Aug 11, 1am (Asia/Tokyo)" had no room left to wrap in a popup
        // that is at most 420px wide.
        column = column
            .push(
                widget::Row::new()
                    .spacing(8)
                    .push(
                        widget::text::body(window.window_label(fallback))
                            .width(Length::Fixed(80.0)),
                    )
                    .push(widget::text::body(format!("{percent:.0}%"))),
            )
            .push(widget::determinate_linear(
                config.usage_display.fraction(window.fraction()),
            ));
        if config.show_reset_countdown
            && let Some(reset) = window.reset_text(now)
        {
            column = column.push(widget::text::caption(reset).width(Length::Fill));
        }
        if config.show_pace
            && let Some(pace) = pace
        {
            for line in pace.summary_lines() {
                column = column.push(widget::text::caption(line).width(Length::Fill));
            }
        }
    }

    if !any {
        column = column.push(widget::text::body("No limit windows reported."));
    }

    if config.show_credits
        && let Some(remaining) = payload.credits.as_ref().and_then(|c| c.remaining)
    {
        column = column.push(widget::text::caption(format!("Credits: {remaining:.2}")));
    }

    column.into()
}

fn header<'a>(payload: &'a ProviderPayload, config: &Config) -> Element<'a, Message> {
    let mut row = widget::Row::new()
        .spacing(8)
        .push(widget::text::title4(payload.label()));
    if config.show_account
        && let Some(account) = &payload.account
    {
        row = row.push(widget::text::caption(account.clone()));
    }
    row.into()
}
