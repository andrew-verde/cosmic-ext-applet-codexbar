use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cosmic::app::Core;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::padded_control;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Container;
use cosmic::iced::{
    Border, Color, Length, Limits, Point, Rectangle, Shadow, Size, Subscription,
    platform_specific::shell::wayland::commands::{
        blur::blur,
        popup::{destroy_popup, get_popup},
    },
    time,
    window::Id,
};
use cosmic::widget;
use cosmic::widget::autosize::{Autosize, autosize};
use cosmic::{Action, Application, Element, Renderer, Task};

use crate::codexbar::{
    CostPayload, PaceWindow, ProviderPayload, RateLimitWindow, fetch_cost, fetch_usage,
    format_cost, format_tokens,
};
use crate::config::Config;
use crate::icons::provider_icon;

const ID: &str = "io.github.andrew_verde.codexbar-cosmic-applet";
const ICON: &str = "io.github.andrew_verde.codexbar-cosmic-applet-symbolic";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Height the scrolling body is capped at. The tab bar, padding and this must
/// stay inside [`popup_limits`]'s `max_height`, which bounds the whole popup.
const MAX_BODY_HEIGHT: f32 = 460.0;

/// Edge length of a provider icon in the tab strip.
const TAB_ICON_SIZE: u16 = 18;

/// Edge length of a provider icon beside the provider name.
const HEADER_ICON_SIZE: u16 = 24;

/// Icon for the Overview tab: four rounded squares in a 2x2 grid.
///
/// Bundled rather than resolved by name. `view-grid-symbolic` used to be looked
/// up through the active icon theme, but themes disagree about what that name
/// draws - Papirus renders a 3x3 pattern of dots - so the tab's look was at the
/// mercy of whatever the user happened to have installed. This is original
/// artwork, not a copy of any theme's file, so it carries no license of its own
/// beyond this crate's.
const OVERVIEW_ICON: &[u8] = include_bytes!("../data/icons/overview-symbolic.svg");

/// Labels for the primary/secondary/tertiary windows when the provider reports
/// no window length to derive one from. Named here because both tabs use them
/// and `UsageSnapshot::window_label_overrides` has to judge a label collision
/// against the same text the rows would actually show.
const SLOT_FALLBACKS: [&str; 3] = ["Session", "Weekly", "Monthly"];

/// Gap between the major blocks of a provider's tab (header, each rate limit
/// window, cost). The macOS app leans on whitespace to separate these.
const BLOCK_SPACING: u16 = 14;

/// Gap between an Overview row's header (provider name, account) and its bars.
/// Wider than the gap between the bars themselves so the two read as separate
/// groups rather than one evenly spaced stack.
const SUMMARY_HEADER_SPACING: u16 = 10;

/// Gap between the session and weekly bar groups of one Overview row.
const SUMMARY_BAR_SPACING: u16 = 6;

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
    TabSelected(Tab),
}

/// Which page of the popup is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
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
    tab: Tab,
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
            costs: Vec::new(),
            tab: Tab::Overview,
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
                // No main window means the panel has not mapped the applet yet,
                // so there is nothing to anchor a popup to.
                let Some(parent) = self.core.main_window_id() else {
                    return Task::none();
                };
                let new_id = Id::unique();
                self.popup.replace(new_id);
                let mut popup_settings = self
                    .core
                    .applet
                    .get_popup_settings(parent, new_id, None, None, None);
                popup_settings.positioner.size_limits = popup_limits();
                return Task::batch([
                    refresh_task(),
                    get_popup(popup_settings),
                    blur_popup(new_id),
                ]);
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
                // Fall back to Overview when the selected provider disappears.
                if let Tab::Provider(provider) = &self.tab
                    && !payloads.iter().any(|p| &p.provider == provider)
                {
                    self.tab = Tab::Overview;
                }
                self.state = State::Loaded(payloads);
            }
            Message::UsageFetched(Err(error)) => self.state = State::Failed(error),
            // Cost is supplementary: a failure just leaves the block out.
            Message::CostFetched(Ok(costs)) => self.costs = costs,
            Message::CostFetched(Err(_)) => self.costs.clear(),
            Message::TabSelected(tab) => self.tab = tab,
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

    /// Every COSMIC applet paints its surface transparent and lets
    /// `popup_container` draw the panel itself; without this the runtime falls
    /// back to a window style meant for ordinary app windows.
    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
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
            State::Loaded(payloads) => match &self.tab {
                Tab::Provider(provider) => {
                    match payloads.iter().find(|p| &p.provider == provider) {
                        Some(payload) => widget::Column::new()
                            .push(self.provider_detail(payload))
                            .spacing(12),
                        None => widget::Column::new()
                            .push(widget::text::body("This provider is no longer reported.")),
                    }
                }
                Tab::Overview => {
                    let mut column = widget::Column::new().spacing(12);
                    for payload in payloads {
                        column = column.push(self.provider_summary(payload));
                    }
                    column
                }
            },
        };

        // The tab strip stays put while only the body scrolls, and the body is
        // capped so long provider lists scroll instead of growing the popup
        // past `popup_limits`'s max height (where they would be clipped).
        let mut content = widget::Column::new().spacing(8);
        if let State::Loaded(payloads) = &self.state
            && !payloads.is_empty()
        {
            content = content.push(self.tab_strip(payloads));
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

    /// Overview plus one tab per provider, scrolling horizontally so the strip
    /// can be wider than the popup instead of cramming labels.
    ///
    /// `segmented_control` distributes its buttons across the available width,
    /// which is what truncated the labels at four tabs; a plain row of buttons
    /// sizes to its content and lets each tab stack its icon over its name.
    fn tab_strip<'a>(&self, payloads: &'a [ProviderPayload]) -> Element<'a, Message> {
        let mut row = widget::Row::new().spacing(4).push(self.tab_button(
            "Overview",
            Some(widget::icon::from_svg_bytes(OVERVIEW_ICON).symbolic(true)),
            Tab::Overview,
        ));
        for payload in payloads {
            row = row.push(self.tab_button(
                payload.label(),
                provider_glyph(&payload.provider),
                Tab::Provider(payload.provider.clone()),
            ));
        }
        widget::scrollable::horizontal(row).into()
    }

    /// One tab: the provider's icon over its name, or name-only when no icon is
    /// vendored for that provider id.
    fn tab_button<'a>(
        &self,
        label: impl Into<String>,
        icon: Option<widget::icon::Handle>,
        tab: Tab,
    ) -> Element<'a, Message> {
        let mut column = widget::Column::new()
            .spacing(2)
            .align_x(Horizontal::Center);
        if let Some(icon) = icon {
            column = column.push(glyph(icon, TAB_ICON_SIZE));
        }
        column = column.push(widget::text::caption(label.into()));

        let selected = self.tab == tab;
        widget::button::custom(column)
            .class(if selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Text
            })
            .padding([4, 8])
            .on_press(Message::TabSelected(tab))
            .into()
    }

    /// One condensed line per provider for the Overview tab: enough to scan
    /// many providers at once without the full per-window layout.
    fn provider_summary<'a>(&'a self, payload: &'a ProviderPayload) -> Element<'a, Message> {
        let mut title = widget::Row::new().spacing(8).align_y(Vertical::Center);
        if let Some(icon) = provider_glyph(&payload.provider) {
            title = title.push(glyph(icon, HEADER_ICON_SIZE));
        }
        title = title.push(widget::text::title3(payload.label()));

        // Spacing here is deliberately uneven: the account line belongs to the
        // header, so the bars below it get a wider gap, while the bars
        // themselves stay tightly grouped (see `bars` below).
        let column = widget::Column::new()
            .spacing(SUMMARY_HEADER_SPACING)
            .width(Length::Fill)
            .push(split_row(title, account_caption(payload, &self.config)));

        if let Some(error) = &payload.error {
            return column
                .push(widget::text::caption(error.message.clone()).width(Length::Fill))
                .into();
        }

        // Session above weekly, each drawn only when the provider reports it
        // (Codex often has no session window), so a provider that starts
        // reporting one picks up its bar with no code change. Monthly stays out
        // of the Overview tab.
        let mut bars = widget::Column::new()
            .spacing(SUMMARY_BAR_SPACING)
            .width(Length::Fill);
        let mut any = false;
        if let Some(usage) = &payload.usage {
            let [primary_label, secondary_label, _] = usage.window_label_overrides(SLOT_FALLBACKS);
            for (window, label, fallback) in [
                (usage.primary.as_ref(), primary_label, "Session"),
                (usage.secondary.as_ref(), secondary_label, "Weekly"),
            ] {
                let Some(window) = window else { continue };
                any = true;
                let label = label.unwrap_or_else(|| window.window_label(fallback));
                bars = bars.push(self.summary_window(window, label));
            }
        }

        if !any {
            return column
                .push(widget::text::caption("No usage data reported.").width(Length::Fill))
                .into();
        }

        column.push(bars).into()
    }

    /// One bar and its percentage/label line for the Overview tab, kept tight
    /// enough to read as a single unit.
    fn summary_window<'a>(
        &'a self,
        window: &'a RateLimitWindow,
        label: String,
    ) -> Element<'a, Message> {
        widget::Column::new()
            .spacing(2)
            .width(Length::Fill)
            .push(widget::determinate_linear(
                self.config.usage_display.fraction(window.fraction()),
            ))
            .push(split_row(
                widget::text::body(self.percent_text(window)),
                widget::text::caption(label),
            ))
            .into()
    }

    /// The full macOS-style layout for one provider.
    fn provider_detail<'a>(&'a self, payload: &'a ProviderPayload) -> Element<'a, Message> {
        let now = Utc::now();
        let mut title = widget::Row::new().spacing(8).align_y(Vertical::Center);
        if let Some(icon) = provider_glyph(&payload.provider) {
            title = title.push(glyph(icon, HEADER_ICON_SIZE));
        }
        title = title.push(widget::text::title3(payload.label()));

        // The header's two rows belong together, so they are their own column;
        // the outer spacing is what separates the major blocks.
        let mut header = widget::Column::new()
            .spacing(2)
            .width(Length::Fill)
            .push(split_row(title, account_caption(payload, &self.config)));

        let mut column = widget::Column::new().spacing(BLOCK_SPACING).width(Length::Fill);

        if let Some(error) = &payload.error {
            return column
                .push(header)
                .push(widget::text::body(error.message.clone()).width(Length::Fill))
                .into();
        }

        let Some(usage) = &payload.usage else {
            return column
                .push(header)
                .push(widget::text::body("No usage data reported.").width(Length::Fill))
                .into();
        };

        header = header.push(split_row(
            widget::text::caption(usage.updated_text(now).unwrap_or_default()),
            widget::text::caption(usage.plan_label().unwrap_or_default()),
        ));
        column = column.push(header);

        let pace = payload.pace.as_ref();
        let [primary_label, secondary_label, tertiary_label] =
            usage.window_label_overrides(SLOT_FALLBACKS);
        let windows = [
            (
                usage.primary.as_ref(),
                primary_label,
                "Session",
                self.config.show_session,
                pace.and_then(|p| p.primary.as_ref()),
            ),
            (
                usage.secondary.as_ref(),
                secondary_label,
                "Weekly",
                self.config.show_weekly,
                pace.and_then(|p| p.secondary.as_ref()),
            ),
            (
                usage.tertiary.as_ref(),
                tertiary_label,
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
        for (window, label, fallback, show, pace) in windows {
            let Some(window) = window else { continue };
            any = true;
            if show {
                let label = label.unwrap_or_else(|| window.window_label(fallback));
                column = column.push(self.window_block(window, pace, label, now));
            }
        }

        if !any {
            column = column.push(widget::text::body("No limit windows reported.").width(Length::Fill));
        }

        // Provider-level, not per-window: a reset credit resets the weekly
        // window, but the grant sits beside the windows rather than inside one.
        if self.config.show_reset_credits
            && let Some(text) = usage.reset_credits_text(now)
        {
            column = column.push(widget::text::caption(text).width(Length::Fill));
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

    /// Section title opposite the reset countdown, the progress bar, the
    /// percentage, then the pace line.
    ///
    /// Only the title row is two-column, and both of its cells come from a
    /// short, bounded vocabulary. The percentage and the pace line each own a
    /// full-width row instead of competing with a sibling: this popup is 320px
    /// wide at its narrowest, which is not enough for two variable-length
    /// strings side by side, and a starved cell word-wraps to one word per line
    /// with the space at each wrap point undrawn. See the layout tests below.
    fn window_block<'a>(
        &'a self,
        window: &'a RateLimitWindow,
        pace: Option<&'a PaceWindow>,
        label: String,
        now: DateTime<Utc>,
    ) -> Element<'a, Message> {
        let reset = if self.config.show_reset_countdown {
            window.reset_text(now)
        } else {
            None
        };

        let mut column = widget::Column::new()
            .spacing(6)
            .width(Length::Fill)
            .push(split_row(
                widget::text::heading(label),
                widget::text::caption(reset.unwrap_or_default()),
            ))
            .push(widget::determinate_linear(
                self.config.usage_display.fraction(window.fraction()),
            ))
            .push(widget::text::heading(self.percent_text(window)).width(Length::Fill));

        if self.config.show_pace
            && let Some(pace) = pace
        {
            let line = pace_line(pace);
            if !line.is_empty() {
                column = column.push(widget::text::caption(line).width(Length::Fill));
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

    /// The popup panel.
    ///
    /// With no `background_opacity` configured this is upstream's
    /// `cosmic::applet::Context::popup_container` verbatim, so the popup is
    /// styled exactly like every other COSMIC applet popup - including the
    /// theme's translucent background when frosted applets are enabled, which
    /// the compositor blurs behind. Only an explicit opacity override falls
    /// through to the local copy below.
    fn popup_container<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Autosize<'a, Message, cosmic::Theme, Renderer> {
        match self.config.background_opacity {
            Some(opacity) => self.popup_container_with_opacity(content, opacity),
            None => self.core.applet.popup_container(content),
        }
    }

    /// `popup_container` with the background alpha replaced outright. The alpha
    /// is absolute rather than a multiplier, so `1.0` gives a solid, readable
    /// panel even when the theme would otherwise render it translucent.
    ///
    /// This is a derivative of `cosmic::applet::Context::popup_container`
    /// (`src/applet/mod.rs` in <https://github.com/pop-os/libcosmic>), which is
    /// MPL-2.0 rather than MIT like the rest of this crate; upstream bakes its
    /// container style in with no hook to override, so there is no way to reach
    /// the alpha without restating it. See `THIRD_PARTY_LICENSES.md`.
    fn popup_container_with_opacity<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
        opacity: f32,
    ) -> Autosize<'a, Message, cosmic::Theme, Renderer> {
        let (vertical_align, horizontal_align) = match self.core.applet.anchor {
            PanelAnchor::Left => (Vertical::Center, Horizontal::Left),
            PanelAnchor::Right => (Vertical::Center, Horizontal::Right),
            PanelAnchor::Top => (Vertical::Top, Horizontal::Center),
            PanelAnchor::Bottom => (Vertical::Bottom, Horizontal::Center),
        };
        autosize(
            Container::<Message, _, Renderer>::new(
                Container::<Message, _, Renderer>::new(content).style(move |theme| {
                    let cosmic = theme.cosmic();
                    let background = cosmic.background(theme.transparent);
                    let mut bg = Color::from(background.base);
                    bg.a = opacity;
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

/// Ask the compositor to blur behind the popup.
///
/// libcosmic issues this for surfaces it tracks itself, but not for popups an
/// applet creates with `get_popup`, so the theme's translucent popup background
/// ends up with nothing blurred behind it. The request is safe to send blind:
/// when `ext_background_effect_v1` is missing or does not advertise blur, the
/// shell queues or drops it rather than erroring.
fn blur_popup(id: Id) -> Task<Action<Message>> {
    blur(
        id,
        Some(vec![Rectangle::new(Point::ORIGIN, Size::INFINITE)]),
    )
    .discard()
}

fn refresh_task() -> Task<Action<Message>> {
    Task::batch([
        Task::perform(fetch_usage(), |result| {
            Message::UsageFetched(result).into()
        }),
        Task::perform(fetch_cost(), |result| Message::CostFetched(result).into()),
    ])
}

/// A left-aligned label with a counterpart flush against the right edge.
///
/// This cannot guarantee the right cell any particular width, and callers must
/// not assume it does. `Row` defaults to `Length::Shrink`, so
/// `layout::flex::resolve` treats the main axis as compressed and lays every
/// child out in document order, each taking from what the previous one left;
/// no ordering changes that. A cell narrower than its text word-wraps, and
/// cosmic-text does not draw the space at a wrap point, so a starved cell reads
/// as though its spaces had vanished. Only pair strings short enough to sit
/// side by side at the narrowest popup width - `window_block` explains where
/// that bites and the tests below hold the line.
fn split_row<'a>(
    left: impl Into<Element<'a, Message>>,
    right: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::Row::new()
        .width(Length::Fill)
        .spacing(8)
        .align_y(Vertical::Center)
        .push(left)
        .push(
            widget::container(right)
                .width(Length::Fill)
                .align_x(Horizontal::Right),
        )
        .into()
}

/// An icon tinted with the theme's foreground colour. Both the vendored
/// provider SVGs and the system's symbolic icons are monochrome templates, so
/// they carry no colour of their own.
fn glyph<'a>(handle: widget::icon::Handle, size: u16) -> Element<'a, Message> {
    widget::icon(handle)
        .size(size)
        .class(cosmic::theme::Svg::custom(|theme| {
            cosmic::iced::widget::svg::Style {
                color: Some(theme.cosmic().background(theme.transparent).on.into()),
            }
        }))
        .into()
}

/// The vendored icon for a provider id, ready to render.
fn provider_glyph(provider: &str) -> Option<widget::icon::Handle> {
    provider_icon(provider).map(|svg| widget::icon::from_svg_bytes(svg).symbolic(true))
}

/// The pace stage and its projection as one full-width line, e.g.
/// "26% in reserve - Lasts until reset". Joining them means the line can wrap
/// at a word boundary if it has to, rather than being squeezed into a column
/// too narrow to hold either half.
fn pace_line(pace: &PaceWindow) -> String {
    let mut parts = Vec::with_capacity(2);
    parts.extend(pace.stage_text());
    parts.extend(pace.projection_text());
    parts.join(" - ")
}

fn account_caption<'a>(payload: &'a ProviderPayload, config: &Config) -> Element<'a, Message> {
    let account = match (config.show_account, payload.account_text()) {
        (true, Some(account)) => account.to_string(),
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
        .spacing(6)
        .width(Length::Fill)
        .push(split_row(
            widget::text::caption("Today"),
            widget::text::caption("30d cost"),
        ))
        .push(split_row(
            widget::text::heading(money(cost.session_cost_usd)),
            widget::text::heading(money(cost.last30_days_cost_usd)),
        ))
        .push(split_row(
            widget::text::caption("Latest tokens"),
            widget::text::caption("30d tokens"),
        ))
        .push(split_row(
            widget::text::heading(tokens(cost.session_tokens)),
            widget::text::heading(tokens(cost.last30_days_tokens)),
        ))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::Pixels;
    use cosmic::iced::advanced::layout::{Limits, Node};
    use cosmic::iced::advanced::widget::Tree;

    /// A real `cosmic::Renderer`, built without a GPU or a compositor so the
    /// layout pass below measures text with the same font machinery the applet
    /// uses at runtime.
    fn renderer() -> Renderer {
        Renderer::Secondary(iced_tiny_skia::Renderer::new(
            cosmic::font::default(),
            Pixels(14.0),
        ))
    }

    /// Lay `element` out at `width` and return the resulting node.
    fn layout(element: Element<'_, Message>, width: f32) -> Node {
        let mut element = element;
        let mut tree = Tree::new(element.as_widget());
        // The popup body is measured with a shrinking (compressed) width, which
        // is what makes `layout::flex::resolve` lay a row's children out in
        // document order.
        let limits = Limits::NONE.max_width(width).width(Length::Shrink);
        element
            .as_widget_mut()
            .layout(&mut tree, &renderer(), &limits)
    }

    /// Width the right-hand cell of a `split_row` is actually given, using the
    /// same widget pairing `window_block` renders.
    fn right_cell_width(left: &str, right: &str, width: f32) -> f32 {
        let node = layout(
            split_row(widget::text::heading(left), widget::text::caption(right)),
            width,
        );
        node.children()[1].size().width
    }

    /// Natural, unwrapped width of a caption.
    fn caption_width(text: &str) -> f32 {
        layout(widget::text::caption(text).into(), f32::INFINITY)
            .size()
            .width
    }

    /// Width `cosmic::widget::scrollable` reserves for its scrollbar and its
    /// padding, which a window block does not get to use.
    const SCROLLBAR_GUTTER: f32 = 16.0;

    /// The narrowest a window block can be: `popup_limits`'s minimum width less
    /// the chrome between the popup edge and the block, i.e. `padded_control`'s
    /// horizontal padding and the scrollbar gutter.
    fn narrowest_block() -> f32 {
        block_width(320.0)
    }

    /// The width a window block gets inside a popup `width` wide.
    fn block_width(popup: f32) -> f32 {
        popup - f32::from(cosmic::theme::spacing().space_m) * 2.0 - SCROLLBAR_GUTTER
    }

    /// Every window title the applet can put in the left cell, longest first.
    const TITLES: [&str; 4] = ["Monthly", "Session", "Weekly", "Tertiary"];

    /// Every reset string `RateLimitWindow::reset_text` can produce, worst case.
    /// It counts down from `resetsAt`, so these are short and bounded; the last
    /// is the longest fallback that survives `strip_parenthetical` when a
    /// provider reports no `resetsAt`.
    const RESET_TEXTS: [&str; 4] = [
        "Resets in 6d 23h",
        "Resets in 18m",
        "Resetting now",
        "Resets Aug 11, 1am",
    ];

    /// The regression this file kept re-introducing.
    ///
    /// When a `split_row` cell is narrower than its text, cosmic-text word-wraps
    /// it and does not draw the space at each wrap point, so
    /// "Resets 3:50pm (Asia/Tokyo)" rendered as "Resets3:50pm(Asia/Tokyo)".
    /// `split_row` cannot guarantee a cell's width on its own: with a compressed
    /// main axis `layout::flex::resolve` lays children out in document order and
    /// the first one takes whatever it asks for. So the invariant is that both
    /// cells of a two-column row come from bounded vocabularies that fit side by
    /// side at the narrowest popup width - which is what this asserts, through
    /// iced's real layout pass with real font metrics.
    #[test]
    fn window_title_row_fits_at_the_narrowest_popup_width() {
        for title in TITLES {
            for reset in RESET_TEXTS {
                let needs = caption_width(reset);
                let available = right_cell_width(title, reset, narrowest_block());
                assert!(
                    available >= needs,
                    "{title:?} + {reset:?}: value needs {needs} but was given {available}"
                );
            }
        }
    }

    /// Height of one line of `widget::text::caption`, per libcosmic's preset.
    const CAPTION_LINE: f32 = 17.0;

    /// Height of one line of `widget::text::heading`.
    const HEADING_LINE: f32 = 21.0;

    /// The pace stage and its projection are two variable-length strings that
    /// together do not fit side by side at the narrowest popup width, so they
    /// share one full-width line instead of a two-column row - which leaves
    /// enough room to set them on a single line rather than one word per line.
    #[test]
    fn pace_line_sets_on_one_line() {
        let pace = PaceWindow {
            stage: None,
            delta_percent: None,
            expected_used_percent: None,
            will_last_to_reset: Some(true),
            eta_seconds: None,
            summary: Some("26% in reserve | Expected 49% used | Lasts until reset".to_string()),
        };
        assert_eq!(pace_line(&pace), "26% in reserve - Lasts until reset");

        // One line at the popup's full width, and never worse than a single
        // word-boundary wrap at its narrowest. The starved rendering this
        // guards against was one word per line, i.e. five.
        let widest = layout(
            widget::text::caption(pace_line(&pace))
                .width(Length::Fill)
                .into(),
            block_width(420.0),
        );
        assert!(
            widest.size().height <= CAPTION_LINE + 1.0,
            "pace line wrapped at full width: {widest:?}"
        );

        let narrowest = layout(
            widget::text::caption(pace_line(&pace))
                .width(Length::Fill)
                .into(),
            narrowest_block(),
        );
        assert!(
            narrowest.size().height <= CAPTION_LINE * 2.0 + 1.0,
            "pace line wrapped more than once: {narrowest:?}"
        );
    }

    /// `show_account = false` still blanks the caption now that there is a real
    /// email behind it - the toggle is what a streamer hides their address with.
    #[test]
    fn show_account_toggles_the_identity_email() {
        let payload = &crate::codexbar::parse_usage_json(
            r#"[{"provider": "codex", "usage":
                 {"identity": {"accountEmail": "redacted@example.com"}}}]"#,
        )
        .unwrap()[0];

        let shown = layout(account_caption(payload, &Config::default()), f32::INFINITY);
        assert!(shown.size().width > 0.0);
        assert_eq!(
            shown.size().width,
            caption_width("redacted@example.com"),
            "the caption should render the identity email"
        );

        let hidden = Config {
            show_account: false,
            ..Config::default()
        };
        assert_eq!(
            layout(account_caption(payload, &hidden), f32::INFINITY)
                .size()
                .width,
            0.0
        );
    }

    /// The Overview tab's glyph is bundled, so it cannot go missing the way a
    /// theme lookup could.
    #[test]
    fn overview_icon_is_a_bundled_svg() {
        assert!(OVERVIEW_ICON.windows(4).any(|w| w == b"<svg"));
    }

    /// The percentage owns its row, so it is never squeezed either.
    #[test]
    fn percentage_sets_on_one_line() {
        let node = layout(
            widget::text::heading("100% remaining")
                .width(Length::Fill)
                .into(),
            narrowest_block(),
        );
        assert!(
            node.size().height <= HEADING_LINE + 1.0,
            "percentage wrapped: {node:?}"
        );
    }
}

