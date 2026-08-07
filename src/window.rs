use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
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
use cosmic::widget::segmented_button::{Entity, SingleSelectModel};
use cosmic::{Action, Application, Element, Renderer, Task};

use crate::codexbar::{
    CostPayload, PaceWindow, ProviderPayload, RateLimitWindow, fetch_cost, fetch_usage,
    format_cost, format_tokens,
};
use crate::config::Config;

const ID: &str = "io.github.andrew-verde.CodexBarCosmicApplet";
const ICON: &str = "io.github.andrew-verde.CodexBarCosmicApplet-symbolic";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Height the scrolling body is capped at. The tab bar, padding and this must
/// stay inside [`popup_limits`]'s `max_height`, which bounds the whole popup.
const MAX_BODY_HEIGHT: f32 = 460.0;

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
    CostFetched(Result<Vec<CostPayload>, String>),
    TabSelected(Entity),
}

/// Which page of the popup is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tab {
    /// One condensed line per provider.
    Overview,
    /// The full layout for a single provider, keyed by provider id.
    Provider(String),
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
    /// Cost data, keyed by provider id. Empty when the `cost` subcommand is
    /// unavailable or reports nothing, which only hides the cost block.
    costs: Vec<CostPayload>,
    tabs: SingleSelectModel,
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
        let mut window = Window {
            core,
            popup: None,
            state: State::Loading,
            costs: Vec::new(),
            tabs: SingleSelectModel::default(),
            config,
            config_error,
        };
        window.rebuild_tabs(&[]);
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
            Message::UsageFetched(Ok(payloads)) => {
                self.rebuild_tabs(&payloads);
                self.state = State::Loaded(payloads);
            }
            Message::UsageFetched(Err(error)) => self.state = State::Failed(error),
            // Cost is supplementary: a failure just leaves the block out.
            Message::CostFetched(Ok(costs)) => self.costs = costs,
            Message::CostFetched(Err(_)) => self.costs.clear(),
            Message::TabSelected(entity) => self.tabs.activate(entity),
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
        let body = match &self.state {
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
            State::Loaded(payloads) => match self.tabs.active_data::<Tab>() {
                Some(Tab::Provider(provider)) => {
                    match payloads.iter().find(|p| &p.provider == provider) {
                        Some(payload) => widget::Column::new()
                            .push(self.provider_detail(payload))
                            .spacing(12),
                        None => widget::Column::new()
                            .push(widget::text::body("This provider is no longer reported.")),
                    }
                }
                _ => {
                    let mut column = widget::Column::new().spacing(12);
                    for payload in payloads {
                        column = column.push(self.provider_summary(payload));
                    }
                    column
                }
            },
        };

        // The tab bar stays put while only the body scrolls, and the body is
        // capped so long provider lists scroll instead of growing the popup
        // past `popup_limits`'s max height (where they would be clipped).
        let mut content = widget::Column::new().spacing(8);
        if self.tabs.len() > 1 {
            content = content.push(
                widget::segmented_control::horizontal(&self.tabs)
                    .on_activate(Message::TabSelected),
            );
        }
        content = content.push(
            widget::container(widget::scrollable(body.width(Length::Fill)))
                .max_height(MAX_BODY_HEIGHT)
                .width(Length::Fill),
        );
        if let Some(error) = &self.config_error {
            content = content.push(widget::text::caption(error.clone()));
        }

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

    /// Rebuild the tab bar for the current provider list, keeping the selected
    /// tab if it still exists and falling back to Overview if it does not.
    fn rebuild_tabs(&mut self, payloads: &[ProviderPayload]) {
        let selected = self.tabs.active_data::<Tab>().cloned();
        let mut tabs = SingleSelectModel::default();
        tabs.insert().text("Overview").data(Tab::Overview).activate();
        for payload in payloads {
            let tab = Tab::Provider(payload.provider.clone());
            let entity = tabs.insert().text(payload.label()).data(tab.clone()).id();
            if selected.as_ref() == Some(&tab) {
                tabs.activate(entity);
            }
        }
        self.tabs = tabs;
    }

    /// One condensed line per provider for the Overview tab: enough to scan
    /// many providers at once without the full per-window layout.
    fn provider_summary<'a>(&'a self, payload: &'a ProviderPayload) -> Element<'a, Message> {
        let mut column = widget::Column::new()
            .spacing(4)
            .push(split_row(
                widget::text::body(payload.label()),
                account_caption(payload, &self.config),
            ));

        if let Some(error) = &payload.error {
            return column
                .push(widget::text::caption(error.message.clone()).width(Length::Fill))
                .into();
        }

        // The shortest window is the most urgent one; providers without it
        // (Codex often has no session window) fall back to the weekly figure.
        let headline = payload.usage.as_ref().and_then(|usage| {
            usage
                .primary
                .as_ref()
                .or(usage.secondary.as_ref())
                .or(usage.tertiary.as_ref())
        });
        let Some(window) = headline else {
            return column
                .push(widget::text::caption("No usage data reported.").width(Length::Fill))
                .into();
        };

        column = column
            .push(widget::determinate_linear(
                self.config.usage_display.fraction(window.fraction()),
            ))
            .push(split_row(
                widget::text::caption(self.percent_text(window)),
                widget::text::caption(window.window_label("Usage")),
            ));
        column.into()
    }

    /// The full macOS-style layout for one provider.
    fn provider_detail<'a>(&'a self, payload: &'a ProviderPayload) -> Element<'a, Message> {
        let now = Utc::now();
        let mut column = widget::Column::new().spacing(4).push(split_row(
            widget::text::title4(payload.label()),
            account_caption(payload, &self.config),
        ));

        if let Some(error) = &payload.error {
            return column
                .push(widget::text::body(error.message.clone()).width(Length::Fill))
                .into();
        }

        let Some(usage) = &payload.usage else {
            return column
                .push(widget::text::body("No usage data reported.").width(Length::Fill))
                .into();
        };

        column = column.push(split_row(
            widget::text::caption(usage.updated_text(now).unwrap_or_default()),
            widget::text::caption(usage.plan_label().unwrap_or_default()),
        ));

        let pace = payload.pace.as_ref();
        let windows = [
            (
                usage.primary.as_ref(),
                "Session",
                self.config.show_session,
                pace.and_then(|p| p.primary.as_ref()),
            ),
            (
                usage.secondary.as_ref(),
                "Weekly",
                self.config.show_weekly,
                pace.and_then(|p| p.secondary.as_ref()),
            ),
            (
                usage.tertiary.as_ref(),
                "Monthly",
                self.config.show_monthly,
                pace.and_then(|p| p.tertiary.as_ref()),
            ),
        ];

        // `any` tracks whether the *data* is present, not whether it is shown,
        // so hiding every window with the config still leaves the provider
        // silent rather than claiming nothing was reported. A window the
        // provider does not report (Codex frequently has no session window) is
        // simply skipped, never drawn as an empty placeholder.
        let mut any = false;
        for (window, fallback, show, pace) in windows {
            let Some(window) = window else { continue };
            any = true;
            if show {
                column = column.push(self.window_block(window, pace, fallback, now));
            }
        }

        if !any {
            column = column.push(widget::text::body("No limit windows reported.").width(Length::Fill));
        }

        if self.config.show_cost
            && let Some(cost) = self.cost_for(&payload.provider)
        {
            column = column.push(cost_block(cost));
        }

        if self.config.show_credits
            && let Some(remaining) = payload.credits.as_ref().and_then(|c| c.remaining)
        {
            column = column.push(widget::text::caption(format!("Credits: {remaining:.2}")));
        }

        column.into()
    }

    /// Title, progress bar, then two two-column caption rows: the percentage
    /// opposite the reset countdown, and the pace stage opposite the projection.
    fn window_block<'a>(
        &'a self,
        window: &'a RateLimitWindow,
        pace: Option<&'a PaceWindow>,
        fallback: &str,
        now: DateTime<Utc>,
    ) -> Element<'a, Message> {
        let reset = if self.config.show_reset_countdown {
            window.reset_text(now)
        } else {
            None
        };

        let mut column = widget::Column::new()
            .spacing(4)
            .push(widget::text::heading(window.window_label(fallback)))
            .push(widget::determinate_linear(
                self.config.usage_display.fraction(window.fraction()),
            ))
            .push(split_row(
                widget::text::body(self.percent_text(window)),
                widget::text::caption(reset.unwrap_or_default()),
            ));

        if self.config.show_pace
            && let Some(pace) = pace
        {
            let stage = pace.stage_text().unwrap_or_default();
            let projection = pace.projection_text().unwrap_or_default();
            if !stage.is_empty() || !projection.is_empty() {
                column = column.push(split_row(
                    widget::text::caption(stage),
                    widget::text::caption(projection),
                ));
            }
        }

        column.into()
    }

    /// "20% used" or "80% remaining", per `usage_display`. The word is part of
    /// the line so the active mode never needs a separate banner.
    fn percent_text(&self, window: &RateLimitWindow) -> String {
        let percent = self
            .config
            .usage_display
            .percent(window.used_percent.unwrap_or(0.0));
        format!("{percent:.0}% {}", self.config.usage_display.label())
    }

    fn cost_for(&self, provider: &str) -> Option<&CostPayload> {
        self.costs
            .iter()
            .find(|cost| cost.provider == provider && cost.has_figures())
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
    Task::batch([
        Task::perform(fetch_usage(), |result| {
            Message::UsageFetched(result).into()
        }),
        Task::perform(fetch_cost(), |result| Message::CostFetched(result).into()),
    ])
}

/// The layout used throughout the popup: a left-aligned item that takes the
/// slack, and a counterpart flush against the right edge.
fn split_row<'a>(
    left: impl Into<Element<'a, Message>>,
    right: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::Row::new()
        .spacing(8)
        .align_y(Vertical::Center)
        .push(widget::container(left).width(Length::Fill))
        .push(right)
        .into()
}

fn account_caption<'a>(payload: &'a ProviderPayload, config: &Config) -> Element<'a, Message> {
    let account = match (config.show_account, &payload.account) {
        (true, Some(account)) => account.clone(),
        _ => String::new(),
    };
    widget::text::caption(account).into()
}

/// "Today" / "30d cost" over their values, then the same for token counts.
fn cost_block<'a>(cost: &'a CostPayload) -> Element<'a, Message> {
    let currency = cost.currency_code.as_deref();
    let money = |amount: Option<f64>| match amount {
        Some(amount) => format_cost(amount, currency),
        None => String::new(),
    };
    let tokens = |count: Option<u64>| match count {
        Some(count) => format_tokens(count),
        None => String::new(),
    };

    widget::Column::new()
        .spacing(4)
        .push(split_row(
            widget::text::caption("Today"),
            widget::text::caption("30d cost"),
        ))
        .push(split_row(
            widget::text::body(money(cost.session_cost_usd)),
            widget::text::body(money(cost.last30_days_cost_usd)),
        ))
        .push(split_row(
            widget::text::caption("Latest tokens"),
            widget::text::caption("30d tokens"),
        ))
        .push(split_row(
            widget::text::body(tokens(cost.session_tokens)),
            widget::text::body(tokens(cost.last30_days_tokens)),
        ))
        .into()
}
