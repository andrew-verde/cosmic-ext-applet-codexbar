use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cosmic::app::Core;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::padded_control;
use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit};
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

const ID: &str = "io.github.andrew_verde.cosmic-ext-applet-codexbar";
const ICON: &str = "io.github.andrew_verde.cosmic-ext-applet-codexbar-symbolic";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Height the scrolling body is capped at. The tab bar, padding and this must
/// stay inside [`popup_limits`]'s `max_height`, which bounds the whole popup.
const MAX_BODY_HEIGHT: f32 = 460.0;

/// Gap between the body and its scrollbar. With libcosmic's 8px bar this makes
/// a 16px gutter that the layout tests below reserve as `SCROLLBAR_GUTTER`.
const SCROLLBAR_SPACING: f32 = 8.0;

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

/// Gear for the settings button. Original artwork like [`OVERVIEW_ICON`],
/// vendored so it does not depend on the icon themes the sandbox can see.
const SETTINGS_ICON: &[u8] = include_bytes!("../data/icons/settings-symbolic.svg");

/// Gap between the major blocks of a provider's tab (header, each rate limit
/// window, cost). The macOS app leans on whitespace to separate these.
const BLOCK_SPACING: u16 = 14;

/// Gap between an Overview row's header (provider name, account) and its bars.
/// Wider than the gap between the bars themselves so the two read as separate
/// groups rather than one evenly spaced stack.
const SUMMARY_HEADER_SPACING: u16 = 10;

/// Gap between the session and weekly bar groups of one Overview row.
const SUMMARY_BAR_SPACING: u16 = 6;

/// Shortens an email that does not fit to one line, keeping both ends
/// readable, e.g. "personal@exa…mple.com".
const ONE_LINE_MIDDLE: Ellipsize = Ellipsize::Middle(EllipsizeHeightLimit::Lines(1));

/// Edge length of the expand/collapse chevron on an account row.
const CHEVRON_SIZE: u16 = 16;

/// Length of the bar summarizing a collapsed account row.
const MINI_BAR_WIDTH: f32 = 56.0;

/// Width reserved for a collapsed row's percentage, so "100%" fits and the
/// bars of stacked rows line up.
const MINI_PERCENT_WIDTH: f32 = 36.0;

/// Padding inside an account row's button, which is what its hover
/// highlight extends to.
const ROW_PADDING: [u16; 2] = [4, 6];

/// Identifies the autosizing popup body to the shell, mirroring the private
/// `AUTOSIZE_ID` that `cosmic::applet::Context::popup_container` uses.
static AUTOSIZE_ID: LazyLock<cosmic::iced::id::Id> =
    LazyLock::new(|| cosmic::iced::id::Id::new("codexbar-applet-autosize"));

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    Refresh,
    UsageFetched(u64, Result<Vec<ProviderPayload>, String>),
    CostFetched(u64, Result<Vec<CostPayload>, String>),
    TabSelected(Tab),
    ToggleAccount(AccountRow),
    EditConfig,
}

/// Which page of the popup is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    /// Accounts grouped under each provider.
    Overview,
    /// The full layout for a single provider, keyed by provider id.
    Provider(String),
}

/// One collapsible account row. Overview and the provider tab track their rows
/// separately, so opening an account in one view leaves the other unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountRow {
    overview: bool,
    provider: String,
    /// The account's email or CLI label, so a row keeps its state across
    /// refreshes. Accounts with neither fall back to their position.
    account: String,
}

impl AccountRow {
    fn new(payload: &ProviderPayload, index: usize, overview: bool) -> Self {
        Self {
            overview,
            provider: payload.provider.clone(),
            account: payload
                .account_text()
                .map_or_else(|| format!("#{index}"), str::to_owned),
        }
    }
}

enum State {
    Loading,
    Loaded(Vec<ProviderPayload>),
    Failed(String),
}

impl State {
    fn apply_usage(&mut self, result: Result<Vec<ProviderPayload>, String>) {
        match result {
            Ok(payloads) => *self = Self::Loaded(payloads),
            // A transient failure should not erase the last successful usage.
            Err(error) if !matches!(self, Self::Loaded(_)) => *self = Self::Failed(error),
            Err(_) => {}
        }
    }
}

/// A refresh owns both CLI results until they finish, including failures.
#[derive(Default)]
struct Refresh {
    generation: u64,
    usage_pending: bool,
    cost_pending: bool,
}

impl Refresh {
    fn start(&mut self) -> Option<u64> {
        if self.usage_pending || self.cost_pending {
            return None;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("refresh generation overflow");
        self.usage_pending = true;
        self.cost_pending = true;
        Some(self.generation)
    }

    fn complete_usage(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.usage_pending {
            return false;
        }
        self.usage_pending = false;
        true
    }

    fn complete_cost(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.cost_pending {
            return false;
        }
        self.cost_pending = false;
        true
    }
}

pub struct Window {
    core: Core,
    popup: Option<Id>,
    state: State,
    refresh: Refresh,
    /// Last successful cost data, keyed by provider id. Empty until the first
    /// successful response, or when that response reports nothing.
    costs: Vec<CostPayload>,
    usage_error: Option<String>,
    cost_error: Option<String>,
    tab: Tab,
    /// Account rows the user has flipped from their default; see `is_open`.
    toggled_accounts: HashSet<AccountRow>,
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
            refresh: Refresh::default(),
            costs: Vec::new(),
            usage_error: None,
            cost_error: None,
            tab: Tab::Overview,
            toggled_accounts: HashSet::new(),
            config,
            config_error,
        };
        let task = window.start_refresh();
        (window, task)
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
                    self.start_refresh(),
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
                return self.start_refresh();
            }
            Message::UsageFetched(generation, result) => {
                if !self.refresh.complete_usage(generation) {
                    return Task::none();
                }
                // Fall back to Overview when the selected provider disappears.
                if let Ok(payloads) = &result
                    && let Tab::Provider(provider) = &self.tab
                    && !payloads.iter().any(|p| &p.provider == provider)
                {
                    self.tab = Tab::Overview;
                }
                self.usage_error = result.as_ref().err().cloned();
                self.state.apply_usage(result);
            }
            Message::CostFetched(generation, result) => {
                if self.refresh.complete_cost(generation) {
                    self.cost_error = result.as_ref().err().cloned();
                    if let Ok(costs) = result {
                        self.costs = costs;
                    }
                }
            }
            Message::TabSelected(tab) => self.tab = tab,
            Message::ToggleAccount(row) => {
                if !self.toggled_accounts.remove(&row) {
                    self.toggled_accounts.insert(row);
                }
            }
            Message::EditConfig => {
                crate::config::open_in_editor();
                if let Some(popup) = self.popup.take() {
                    return destroy_popup(popup);
                }
            }
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
        self.popup_container(padded_control(self.popup_content()))
            .limits(popup_limits())
            .into()
    }
}

impl Window {
    /// Everything inside the popup panel: tabs, notices and the scrolling body.
    fn popup_content(&self) -> Element<'_, Message> {
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
            State::Loaded(payloads) => {
                let groups = group_providers(payloads);
                match &self.tab {
                    Tab::Provider(provider) => {
                        match groups.iter().find(|group| group.provider == provider) {
                            Some(group) => {
                                widget::Column::new().push(self.provider_detail(&group.accounts))
                            }
                            None => widget::Column::new()
                                .push(widget::text::body("This provider is no longer reported.")),
                        }
                    }
                    Tab::Overview => {
                        let mut column = widget::Column::new().spacing(BLOCK_SPACING);
                        for group in &groups {
                            column = column.push(self.provider_summary(&group.accounts));
                        }
                        column
                    }
                }
            }
        };

        // The tab strip stays put while only the body scrolls, and the body is
        // capped so long provider lists scroll instead of growing the popup
        // past `popup_limits`'s max height (where they would be clipped).
        // The settings button keeps its corner even while loading or failed,
        // since a broken config is one reason to reach for it.
        let tabs: Element<'_, Message> = match &self.state {
            State::Loaded(payloads) if !payloads.is_empty() => self.tab_strip(payloads),
            _ => widget::Space::new().width(Length::Fill).into(),
        };
        let settings = widget::button::icon(
            widget::icon::from_svg_bytes(SETTINGS_ICON).symbolic(true),
        )
        .on_press(Message::EditConfig);
        let mut content = widget::Column::new().spacing(8).push(
            widget::Row::new()
                .spacing(4)
                .align_y(Vertical::Center)
                .push(widget::container(tabs).width(Length::Fill))
                .push(settings),
        );
        let mut notices = widget::Column::new().spacing(8);
        if self.usage_error.is_some() && matches!(self.state, State::Loaded(_)) {
            notices = notices.push(widget::text::caption(
                "Last refresh failed. Showing previous usage.",
            ));
        }
        if self.cost_error.is_some() && self.config.show_cost {
            notices = notices.push(widget::text::caption(if self.costs.is_empty() {
                "Cost refresh failed."
            } else {
                "Cost refresh failed. Showing previous cost data."
            }));
        }
        let body = notices.push(body);
        // The scrollbar is embedded so it takes its own gutter when shown,
        // instead of floating over the right-aligned column of text.
        content = content.push(
            widget::container(
                widget::scrollable(body.width(Length::Fill)).spacing(SCROLLBAR_SPACING),
            )
            .max_height(MAX_BODY_HEIGHT)
            .width(Length::Fill),
        );
        if let Some(error) = &self.config_error {
            content = content.push(widget::text::caption(error.clone()));
        }
        content.width(Length::Fill).into()
    }

    fn start_refresh(&mut self) -> Task<Action<Message>> {
        self.refresh
            .start()
            .map(refresh_task)
            .unwrap_or_else(Task::none)
    }

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
        for group in group_providers(payloads) {
            let payload = group.accounts[0];
            row = row.push(self.tab_button(
                payload.label(),
                provider_glyph(&payload.provider),
                Tab::Provider(payload.provider.clone()),
            ));
        }
        // Embedded like the body's scrollbar, so an overflowing strip grows a
        // gutter instead of drawing the bar across the tab names.
        widget::scrollable::horizontal(row)
            .spacing(SCROLLBAR_SPACING)
            .into()
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

    /// One Overview entry. A single account sits under its provider's title;
    /// several become collapsible rows beneath one provider heading.
    fn provider_summary<'a>(&'a self, accounts: &[&'a ProviderPayload]) -> Element<'a, Message> {
        let mut column = widget::Column::new()
            .spacing(SUMMARY_HEADER_SPACING)
            .width(Length::Fill);
        if let [payload] = accounts {
            column = column.push(split_row(
                provider_heading(payload),
                account_caption(payload, &self.config),
            ));
            if let Some(status) = service_status(payload) {
                column = column.push(widget::text::body(status).width(Length::Fill));
            }
            return column.push(self.summary_body(payload)).into();
        }

        column = column.push(provider_heading(accounts[0]));
        if let Some(status) = accounts.iter().find_map(|payload| service_status(payload)) {
            column = column.push(widget::text::body(status).width(Length::Fill));
        }
        for (index, &payload) in accounts.iter().enumerate() {
            let row = AccountRow::new(payload, index, true);
            if self.is_open(&row, index) {
                column = column.push(
                    widget::Column::new()
                        .spacing(SUMMARY_HEADER_SPACING)
                        .width(Length::Fill)
                        .push(self.account_row(payload, index, row))
                        .push(self.summary_body(payload)),
                );
            } else {
                column = column.push(self.account_row(payload, index, row));
            }
        }
        column.into()
    }

    /// One provider tab. A single account keeps the provider's title; several
    /// become collapsible rows, then the provider's one local cost block.
    fn provider_detail<'a>(&'a self, accounts: &[&'a ProviderPayload]) -> Element<'a, Message> {
        let mut column = widget::Column::new()
            .spacing(BLOCK_SPACING)
            .width(Length::Fill);
        if let [payload] = accounts {
            let mut header = widget::Column::new()
                .spacing(2)
                .width(Length::Fill)
                .push(split_row(
                    provider_heading(payload),
                    account_caption(payload, &self.config),
                ));
            if let Some(status) = service_status(payload) {
                header = header.push(widget::text::body(status).width(Length::Fill));
            }
            column = column.push(self.account_detail(payload, header));
        } else {
            column = column.push(provider_heading(accounts[0]));
            if let Some(status) = accounts.iter().find_map(|payload| service_status(payload)) {
                column = column.push(widget::text::body(status).width(Length::Fill));
            }
            for (index, &payload) in accounts.iter().enumerate() {
                let row = AccountRow::new(payload, index, false);
                column = column.push(if self.is_open(&row, index) {
                    let header = widget::Column::new()
                        .spacing(2)
                        .width(Length::Fill)
                        .push(self.account_row(payload, index, row));
                    self.account_detail(payload, header)
                } else {
                    self.account_row(payload, index, row)
                });
            }
        }
        if self.config.show_cost
            && let Some(cost) = self.cost_for(&accounts[0].provider)
        {
            if accounts.len() > 1 {
                column = column.push(widget::divider::horizontal::default()).push(
                    widget::text::caption(format!("{} cost on this machine", accounts[0].label())),
                );
            }
            column = column.push(cost_block(cost));
        }
        column.into()
    }

    /// Whether an account row is expanded: its default, flipped if toggled.
    /// Provider tabs open their first account; Overview starts collapsed.
    fn is_open(&self, row: &AccountRow, index: usize) -> bool {
        (!row.overview && index == 0) != self.toggled_accounts.contains(row)
    }

    /// A clickable account header. Collapsed, it shows a mini bar for the
    /// account's tightest window and summary lines beneath; open, it shows the
    /// account's email and the caller renders the account below it.
    fn account_row<'a>(
        &'a self,
        payload: &'a ProviderPayload,
        index: usize,
        row: AccountRow,
    ) -> Element<'a, Message> {
        let open = self.is_open(&row, index);
        let name = account_label(payload, index, &self.config);
        // The same windows the expanded account would show, so the mini bar
        // never summarizes a window the lines below leave out.
        let windows: Vec<_> = usage_windows(payload)
            .into_iter()
            .filter(|window| {
                if row.overview {
                    window.slot < 2
                } else {
                    self.shows_slot(window.slot)
                }
            })
            .collect();

        let chevron = widget::icon::from_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        })
        .handle();
        let mut title = widget::Row::new()
            .spacing(6)
            .align_y(Vertical::Center)
            .push(glyph(chevron, CHEVRON_SIZE));
        let shows_email = open
            && self.config.show_account
            && payload.account_text().is_some_and(|email| email != name);
        title = title.push(
            widget::text::body(name)
                .width(Length::Fill)
                .ellipsize(ONE_LINE_MIDDLE),
        );
        if shows_email {
            title = title.push(account_caption(payload, &self.config));
        } else if !open
            && payload.error.is_none()
            && let Some(window) = tightest(&windows)
            && let Some(fraction) = window.fraction()
        {
            title = title
                .push(
                    widget::determinate_linear(self.config.usage_display.fraction(fraction))
                        .width(Length::Fixed(MINI_BAR_WIDTH)),
                )
                .push(
                    widget::text::caption(self.short_percent(window))
                        .width(Length::Fixed(MINI_PERCENT_WIDTH))
                        .align_x(Horizontal::Right),
                );
        }

        let mut content = widget::Column::new()
            .spacing(2)
            .width(Length::Fill)
            .push(title);
        if !open {
            for line in self.collapsed_lines(payload, &windows, row.overview) {
                content = content.push(widget::text::caption(line).width(Length::Fill));
            }
        }
        widget::button::custom(content)
            // Plain text at rest with a hover highlight, like an applet menu entry.
            .class(cosmic::theme::Button::AppletMenu)
            .padding(ROW_PADDING)
            .width(Length::Fill)
            .on_press(Message::ToggleAccount(row))
            .into()
    }

    /// What a collapsed account row says beneath its name: every window on
    /// one line in Overview, or one line per window with its reset in a tab.
    fn collapsed_lines(
        &self,
        payload: &ProviderPayload,
        windows: &[UsageWindow<'_>],
        overview: bool,
    ) -> Vec<String> {
        if let Some(error) = &payload.error {
            return vec![error.message.clone()];
        }
        if windows.is_empty() {
            // Windows hidden by the config are not missing data.
            let reported = !usage_windows(payload).is_empty() || provider_details(payload).is_some();
            return if reported {
                Vec::new()
            } else {
                vec!["No usage data reported.".to_string()]
            };
        }
        if overview {
            let parts: Vec<_> = windows
                .iter()
                .map(|window| format!("{} {}", window.label, self.short_percent(window.window)))
                .collect();
            return vec![parts.join(" · ")];
        }
        let now = Utc::now();
        windows
            .iter()
            .map(|window| {
                let mut line = format!("{} {}", window.label, self.percent_text(window.window));
                if self.config.show_reset_countdown
                    && let Some(reset) = window.window.reset_text(now)
                {
                    line = format!("{line} · {reset}");
                }
                line
            })
            .collect()
    }

    /// Session above weekly for one account in Overview, each drawn only when
    /// the provider reports it (Codex often has no session window). Monthly
    /// stays out of the Overview tab.
    fn summary_body<'a>(&'a self, payload: &'a ProviderPayload) -> Element<'a, Message> {
        if let Some(error) = &payload.error {
            return widget::text::caption(error.message.clone())
                .width(Length::Fill)
                .into();
        }
        let windows: Vec<_> = usage_windows(payload)
            .into_iter()
            .filter(|window| window.slot < 2)
            .collect();
        if windows.is_empty() {
            return provider_details(payload).unwrap_or_else(|| {
                widget::text::caption("No usage data reported.")
                    .width(Length::Fill)
                    .into()
            });
        }
        let mut bars = widget::Column::new()
            .spacing(SUMMARY_BAR_SPACING)
            .width(Length::Fill);
        for window in windows {
            bars = bars.push(self.summary_window(window.window, window.label));
        }
        bars.into()
    }

    /// One Overview window: its name beside its percentage, over a full-width
    /// bar, so a name like Antigravity's "Claude and GPT" stays attached to
    /// its number.
    fn summary_window<'a>(
        &'a self,
        window: &'a RateLimitWindow,
        label: String,
    ) -> Element<'a, Message> {
        let mut column = widget::Column::new()
            .spacing(4)
            .width(Length::Fill)
            .push(split_row(
                widget::text::body(label),
                widget::text::body(self.percent_text(window)),
            ));
        if let Some(fraction) = window.fraction() {
            column = column.push(
                widget::determinate_linear(self.config.usage_display.fraction(fraction))
                    .width(Length::Fill),
            );
        }
        column.into()
    }

    /// Usage, pace and credits for one account, below `header`. The
    /// updated/plan line joins `header` so the two read as one block.
    fn account_detail<'a>(
        &'a self,
        payload: &'a ProviderPayload,
        mut header: widget::Column<'a, Message, cosmic::Theme, Renderer>,
    ) -> Element<'a, Message> {
        let now = Utc::now();
        let column = widget::Column::new()
            .spacing(BLOCK_SPACING)
            .width(Length::Fill);

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
            widget::text::caption(payload.plan_label().unwrap_or_default()),
        ));
        let mut column = column.push(header);

        // A window the provider does not report (Codex frequently has no
        // session window) is simply skipped, never drawn as an empty
        // placeholder. Hiding every reported window with the config still
        // leaves the provider silent rather than claiming nothing was reported.
        let windows = usage_windows(payload);
        for window in &windows {
            if self.shows_slot(window.slot) {
                let pace = payload.pace.as_ref().and_then(|pace| {
                    [&pace.primary, &pace.secondary, &pace.tertiary][window.slot].as_ref()
                });
                column = column.push(self.window_block(
                    window.window,
                    pace,
                    window.label.clone(),
                    now,
                ));
            }
        }

        if let Some(details) = provider_details(payload) {
            column = column.push(details);
        } else if windows.is_empty() {
            column = column.push(widget::text::body("No limit windows reported.").width(Length::Fill));
        }

        // Provider-level, not per-window: a reset credit resets the weekly
        // window, but the grant sits beside the windows rather than inside one.
        if self.config.show_reset_credits
            && let Some(text) = usage.reset_credits_text(now)
        {
            column = column.push(widget::text::caption(text).width(Length::Fill));
        }

        if self.config.show_credits
            && let Some(remaining) = payload.credits.as_ref().and_then(|c| c.remaining)
        {
            column = column.push(widget::text::caption(format!("Credits: {remaining:.2}")));
        }

        column.into()
    }

    /// Whether the config shows the window in `slot` on a provider tab.
    fn shows_slot(&self, slot: usize) -> bool {
        [
            self.config.show_session,
            self.config.show_weekly,
            self.config.show_monthly,
        ][slot]
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
            ));
        if let Some(fraction) = window.fraction() {
            column = column
                .push(
                    widget::determinate_linear(self.config.usage_display.fraction(fraction))
                        .width(Length::Fill),
                )
                .push(widget::text::heading(self.percent_text(window)).width(Length::Fill));
        } else {
            column = column
                .push(widget::text::heading("Unavailable"))
                .push(widget::text::caption("No usage percentage reported."));
        }

        if self.config.show_pace
            && window.fraction().is_some()
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
        match self.display_percent(window) {
            Some(percent) => format!("{percent:.0}% {}", self.config.usage_display.label()),
            None => "Unavailable".to_string(),
        }
    }

    /// "20%" without the mode word, for the collapsed account rows where the
    /// word would crowd out the account name.
    fn short_percent(&self, window: &RateLimitWindow) -> String {
        match self.display_percent(window) {
            Some(percent) => format!("{percent:.0}%"),
            None => "?".to_string(),
        }
    }

    /// The percentage to print, used or remaining per `usage_display`.
    fn display_percent(&self, window: &RateLimitWindow) -> Option<f64> {
        let percent = window.used_percent.filter(|percent| percent.is_finite())?;
        Some(self.config.usage_display.percent(percent.clamp(0.0, 100.0)))
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

fn refresh_task(generation: u64) -> Task<Action<Message>> {
    Task::batch([
        Task::perform(fetch_usage(), move |result| {
            Message::UsageFetched(generation, result).into()
        }),
        Task::perform(fetch_cost(), move |result| {
            Message::CostFetched(generation, result).into()
        }),
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

struct ProviderGroup<'a> {
    provider: &'a str,
    accounts: Vec<&'a ProviderPayload>,
}

/// Preserve provider and account order even if payloads are interleaved.
fn group_providers(payloads: &[ProviderPayload]) -> Vec<ProviderGroup<'_>> {
    let mut groups: Vec<ProviderGroup<'_>> = Vec::new();
    for payload in payloads {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.provider == payload.provider)
        {
            group.accounts.push(payload);
        } else {
            groups.push(ProviderGroup {
                provider: &payload.provider,
                accounts: vec![payload],
            });
        }
    }
    groups
}

/// A reported usage window with its display name. `slot` is 0 for primary, 1
/// for secondary and 2 for tertiary, which config toggles and pace follow.
struct UsageWindow<'a> {
    slot: usize,
    label: String,
    window: &'a RateLimitWindow,
}

/// The windows a payload reports, in slot order, named by the CLI's labels or
/// a fallback derived from the window length.
fn usage_windows(payload: &ProviderPayload) -> Vec<UsageWindow<'_>> {
    let Some(usage) = &payload.usage else {
        return Vec::new();
    };
    [
        (&usage.primary, "Session"),
        (&usage.secondary, "Weekly"),
        (&usage.tertiary, "Monthly"),
    ]
    .into_iter()
    .zip(payload.window_labels())
    .enumerate()
    .filter_map(|(slot, ((window, fallback), label))| {
        let window = window.as_ref()?;
        let label = label
            .map(str::to_owned)
            .unwrap_or_else(|| window.window_label(fallback));
        Some(UsageWindow {
            slot,
            label,
            window,
        })
    })
    .collect()
}

/// The window nearest its limit, which a collapsed account row summarizes.
fn tightest<'a>(windows: &[UsageWindow<'a>]) -> Option<&'a RateLimitWindow> {
    windows
        .iter()
        .filter_map(|window| Some((window.window.fraction()?, window.window)))
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, window)| window)
}

fn provider_heading<'a>(payload: &ProviderPayload) -> Element<'a, Message> {
    let mut title = widget::Row::new().spacing(8).align_y(Vertical::Center);
    if let Some(icon) = provider_glyph(&payload.provider) {
        title = title.push(glyph(icon, HEADER_ICON_SIZE));
    }
    title.push(widget::text::title3(payload.label())).into()
}

/// Operational status stays quiet. Unknown upstream indicators remain readable.
fn service_status(payload: &ProviderPayload) -> Option<String> {
    let status = payload.status.as_ref()?;
    let indicator = status.indicator.as_deref().map(str::trim).unwrap_or_default();
    if indicator.eq_ignore_ascii_case("none") {
        return None;
    }
    let description = nonblank(status.description.as_deref())
        .or_else(|| nonblank(Some(indicator)))?;
    let label = match indicator {
        "minor" | "major" | "critical" => "Service issue",
        _ => "Service status",
    };
    Some(format!("{label}: {description}"))
}

fn nonblank(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

/// Render text detail rows, including providers that report no quota windows.
fn provider_details(payload: &ProviderPayload) -> Option<Element<'_, Message>> {
    let sections = payload.usage.as_ref()?.details.as_ref()?;
    let mut column = widget::Column::new().spacing(8).width(Length::Fill);
    let mut any = false;
    for section in sections {
        let rows: Vec<_> = section
            .rows
            .iter()
            .flatten()
            .filter_map(|row| {
                let value = nonblank(row.value.as_deref())
                    .or_else(|| nonblank(row.secondary_value.as_deref()))?;
                let text = match nonblank(row.label.as_deref()) {
                    Some(label) => format!("{label}: {value}"),
                    None => value.to_owned(),
                };
                Some((row, text))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        any = true;
        let mut block = widget::Column::new().spacing(4).width(Length::Fill);
        if let Some(title) = nonblank(section.title.as_deref()) {
            block = block.push(widget::text::heading(title));
        }
        for (row, text) in rows {
            block = block.push(widget::text::body(text).width(Length::Fill));
            if nonblank(row.value.as_deref()).is_some()
                && let Some(secondary) = nonblank(row.secondary_value.as_deref())
            {
                block = block.push(widget::text::caption(secondary).width(Length::Fill));
            }
        }
        column = column.push(block);
    }
    any.then(|| column.into())
}

/// An account row's name: a non-email CLI label, the email when
/// `show_account` allows it, or a numbered "Account N".
fn account_label(payload: &ProviderPayload, index: usize, config: &Config) -> String {
    payload
        .account
        .as_deref()
        .filter(|account| !account.contains('@'))
        .or_else(|| config.show_account.then(|| payload.account_text()).flatten())
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Account {}", index + 1))
}

fn account_caption<'a>(payload: &'a ProviderPayload, config: &Config) -> Element<'a, Message> {
    let account = match (config.show_account, payload.account_text()) {
        (true, Some(account)) => account.to_string(),
        _ => String::new(),
    };
    widget::text::caption(account)
        .ellipsize(ONE_LINE_MIDDLE)
        .into()
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

    #[test]
    fn refresh_gates_overlap_and_rejects_old_or_duplicate_results() {
        let mut refresh = Refresh::default();
        let first = refresh.start().unwrap();
        assert_eq!(refresh.start(), None);
        assert!(refresh.complete_usage(first));
        assert!(!refresh.complete_usage(first));
        assert_eq!(refresh.start(), None);
        assert!(refresh.complete_cost(first));

        let second = refresh.start().unwrap();
        assert_ne!(first, second);
        assert!(!refresh.complete_usage(first));
        assert!(!refresh.complete_cost(first));
        assert_eq!(refresh.start(), None);
        assert!(refresh.complete_cost(second));
        assert_eq!(refresh.start(), None);
        assert!(refresh.complete_usage(second));
        assert!(refresh.start().is_some());
    }

    #[test]
    fn failed_usage_retains_successful_data_and_initial_failure_can_recover() {
        let mut state = State::Loading;
        state.apply_usage(Err("unavailable".into()));
        assert!(matches!(&state, State::Failed(error) if error == "unavailable"));
        let payloads = crate::codexbar::parse_usage_json(
            r#"[{"provider":"codex"}]"#).unwrap();
        state.apply_usage(Ok(payloads));
        state.apply_usage(Err("timed out".into()));
        assert!(matches!(&state, State::Loaded(payloads) if payloads.len() == 1));
        // A successful empty response still reflects provider removal.
        state.apply_usage(Ok(Vec::new()));
        assert!(matches!(&state, State::Loaded(payloads) if payloads.is_empty()));
    }

    #[test]
    fn service_status_hides_operational_and_preserves_future_indicators() {
        let payloads = crate::codexbar::parse_usage_json(
            r#"[
            {"provider":"ok","status":{"indicator":"none","description":"All systems operational"}},
            {"provider":"incident","status":{"indicator":"minor","description":"Degraded performance"}},
            {"provider":"future","status":{"indicator":"future","description":"New status"}},
            {"provider":"empty","status":{}}
        ]"#,
        )
        .unwrap();
        assert_eq!(service_status(&payloads[0]), None);
        assert_eq!(
            service_status(&payloads[1]).as_deref(),
            Some("Service issue: Degraded performance")
        );
        assert_eq!(
            service_status(&payloads[2]).as_deref(),
            Some("Service status: New status")
        );
        assert_eq!(service_status(&payloads[3]), None);
    }

    #[test]
    fn detail_rows_render_without_windows_and_empty_sections_are_skipped() {
        let payloads = crate::codexbar::parse_usage_json(
            r#"[
            {"provider":"details","usage":{"details":[
                {"title":"Balance","rows":[{"label":"Remaining","value":"12.40","secondaryValue":"Workspace balance"}]},
                {"rows":[{"label":"Reporting window","secondaryValue":"Monthly"}]}
            ]}},
            {"provider":"empty","usage":{"details":[{"title":"Empty","rows":[{"label":"No value"}]}]}}
        ]"#,
        )
        .unwrap();
        let rendered = layout(provider_details(&payloads[0]).unwrap(), narrowest_block());
        assert!(rendered.size().height > HEADING_LINE + CAPTION_LINE);
        assert!(rendered.size().width <= narrowest_block());
        assert!(provider_details(&payloads[1]).is_none());
    }

    #[test]
    fn groups_interleaved_accounts_in_provider_order() {
        let payloads = crate::codexbar::parse_usage_json(
            r#"[
            {"provider":"codex","account":"personal@example.com"},
            {"provider":"claude"},
            {"provider":"codex","account":"team@example.com"},
            {"provider":"antigravity"}
        ]"#,
        )
        .unwrap();
        let groups = group_providers(&payloads);
        assert_eq!(
            groups
                .iter()
                .map(|group| group.provider)
                .collect::<Vec<_>>(),
            ["codex", "claude", "antigravity"]
        );
        assert_eq!(
            groups[0]
                .accounts
                .iter()
                .map(|p| p.account_text())
                .collect::<Vec<_>>(),
            [Some("personal@example.com"), Some("team@example.com")]
        );
    }

    #[test]
    fn numbers_accounts_without_exposing_email_when_hidden() {
        let payloads = crate::codexbar::parse_usage_json(
            r#"[
            {"provider":"codex","account":"team@example.com","usage":{"identity":{"accountEmail":"identity@example.com"}}},
            {"provider":"claude","account":"Work"},
            {"provider":"codex","account":"personal@example.com"}
        ]"#,
        )
        .unwrap();
        let config = crate::config::parse_config(
            r#"
            show_account = false
        "#,
        )
        .unwrap();
        assert_eq!(account_label(&payloads[0], 0, &config), "Account 1");
        assert_eq!(account_label(&payloads[1], 1, &config), "Work");
        assert_eq!(account_label(&payloads[2], 2, &config), "Account 3");
        assert_eq!(
            account_label(&payloads[2], 2, &Config::default()),
            "personal@example.com"
        );
        assert_eq!(
            layout(account_caption(&payloads[0], &config), f32::INFINITY)
                .size()
                .width,
            0.0
        );
    }

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

    /// Common CLI labels and slot fallbacks, including Antigravity's pools.
    const TITLES: [&str; 6] = [
        "Gemini Models",
        "Claude and GPT",
        "Monthly",
        "Session",
        "Weekly",
        "Tertiary",
    ];

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

    /// Usage for two Codex accounts, Claude and Antigravity, with resets
    /// relative to `now` so countdowns read as they would live.
    fn sample_payloads(now: DateTime<Utc>) -> Vec<ProviderPayload> {
        let at = |hours: i64| (now + chrono::Duration::hours(hours)).to_rfc3339();
        crate::codexbar::parse_usage_json(&format!(
            r#"[
            {{"provider":"codex","account":"personal@example.com",
              "rateWindowLabels":{{"primary":"Session","secondary":"Weekly"}},
              "pace":{{"secondary":{{"willLastToReset":true,"summary":"7% in reserve | Expected 32% used | Lasts until reset"}}}},
              "credits":{{"remaining":0}},
              "usage":{{"identity":{{"accountEmail":"personal@example.com","loginMethod":"plus"}},
                "updatedAt":"{now}",
                "primary":{{"usedPercent":3,"windowMinutes":300,"resetsAt":"{h5}"}},
                "secondary":{{"usedPercent":25,"windowMinutes":10080,"resetsAt":"{h115}"}}}}}},
            {{"provider":"codex","account":"team@example.org",
              "rateWindowLabels":{{"primary":"Session","secondary":"Weekly"}},
              "usage":{{"identity":{{"accountEmail":"team@example.org","loginMethod":"team"}},
                "updatedAt":"{now}",
                "secondary":{{"usedPercent":0,"windowMinutes":10080,"resetsAt":"{h167}"}}}}}},
            {{"provider":"claude",
              "usage":{{"identity":{{"accountEmail":"personal@example.com"}},"updatedAt":"{now}",
                "primary":{{"usedPercent":2,"windowMinutes":300,"resetsAt":"{h3}"}},
                "secondary":{{"usedPercent":28,"windowMinutes":10080,"resetsAt":"{h80}"}}}}}},
            {{"provider":"antigravity",
              "rateWindowLabels":{{"primary":"Gemini Models","secondary":"Claude and GPT"}},
              "usage":{{"identity":{{"accountEmail":"student@example.ac.jp","loginMethod":"Antigravity Starter Quota"}},
                "updatedAt":"{now}",
                "primary":{{"usedPercent":0,"resetsAt":"{h167}"}},
                "secondary":{{"usedPercent":0,"resetsAt":"{h167}"}}}}}}
        ]"#,
            now = now.to_rfc3339(),
            h3 = at(3),
            h5 = at(5),
            h80 = at(80),
            h115 = at(115),
            h167 = at(167),
        ))
        .unwrap()
    }

    fn sample_window(config: Config) -> Window {
        Window {
            core: Core::default(),
            popup: None,
            state: State::Loaded(sample_payloads(Utc::now())),
            refresh: Refresh::default(),
            costs: crate::codexbar::parse_cost_json(
                r#"[{"provider":"codex","sessionCostUSD":1.2,"sessionTokens":1200000,
                     "last30DaysCostUSD":48.5,"last30DaysTokens":96000000}]"#,
            )
            .unwrap(),
            usage_error: None,
            cost_error: None,
            tab: Tab::Overview,
            toggled_accounts: HashSet::new(),
            config,
            config_error: None,
        }
    }

    /// Draws the popup's content at `width` with the tiny-skia renderer and
    /// writes it to `path` as a PAM image (`magick x.pam x.png` converts it).
    fn render(window: &Window, width: f32, cursor: Option<Point>, path: &std::path::Path) {
        use cosmic::iced::advanced::Layout;
        use cosmic::iced::advanced::renderer::{Headless, Style};
        use cosmic::iced::mouse::Cursor;

        let mut element: Element<'_, Message> = padded_control(window.popup_content()).into();
        let mut tree = Tree::new(element.as_widget());
        let mut renderer = renderer();
        let limits = Limits::new(Size::new(width, 0.0), Size::new(width, 2000.0));
        let node = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        let size = node.size();
        let theme = cosmic::Theme::dark();
        let on = theme.cosmic().background(false).on;
        element.as_widget().draw(
            &tree,
            &mut renderer,
            &theme,
            &Style {
                icon_color: on.into(),
                text_color: on.into(),
                scale_factor: 1.0,
            },
            Layout::new(&node),
            cursor.map_or(Cursor::Unavailable, Cursor::Available),
            &Rectangle::with_size(size),
        );
        let Renderer::Secondary(renderer) = &mut renderer else {
            unreachable!("the test renderer is tiny-skia")
        };
        let (w, h) = (size.width.ceil() as u32, size.height.ceil() as u32);
        let background = Color::from(theme.cosmic().background(false).base);
        let pixels = renderer.screenshot(Size::new(w, h), 1.0, background);
        let mut file = format!(
            "P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
        )
        .into_bytes();
        file.extend(pixels);
        std::fs::write(path, file).unwrap();
    }

    /// Renders the popup states that matter for review into
    /// `$CODEXBAR_RENDER_DIR`. Ignored by default because it produces images
    /// for a person to look at rather than asserting anything:
    ///
    /// ```sh
    /// CODEXBAR_RENDER_DIR=/tmp/render cargo test render_popup_states -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn render_popup_states() {
        let dir = std::path::PathBuf::from(
            std::env::var("CODEXBAR_RENDER_DIR").expect("set CODEXBAR_RENDER_DIR"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let codex = Tab::Provider("codex".to_string());
        let toggle = |window: &mut Window, overview: bool, account: &str| {
            let _ = window.update(Message::ToggleAccount(AccountRow {
                overview,
                provider: "codex".to_string(),
                account: account.to_string(),
            }));
        };

        let mut window = sample_window(Config::default());
        // Over the first Codex account row, to show its hover highlight.
        render(
            &window,
            420.0,
            Some(Point::new(150.0, 125.0)),
            &dir.join("overview-hover.pam"),
        );
        for width in [320.0, 420.0] {
            render(&window, width, None, &dir.join(format!("overview-{width}.pam")));
        }
        toggle(&mut window, true, "team@example.org");
        render(&window, 420.0, None, &dir.join("overview-team-open.pam"));

        window.tab = codex.clone();
        render(&window, 420.0, None, &dir.join("codex.pam"));
        render(&window, 320.0, None, &dir.join("codex-320.pam"));
        toggle(&mut window, false, "personal@example.com");
        toggle(&mut window, false, "team@example.org");
        render(&window, 420.0, None, &dir.join("codex-team-open.pam"));
        // Both accounts open overflows the body, so it scrolls.
        toggle(&mut window, false, "personal@example.com");
        render(&window, 420.0, None, &dir.join("codex-both-open.pam"));

        window.tab = Tab::Provider("antigravity".to_string());
        render(&window, 420.0, None, &dir.join("antigravity.pam"));

        let mut private = sample_window(Config {
            show_account: false,
            usage_display: crate::config::UsageDisplay::Remaining,
            ..Config::default()
        });
        render(&private, 320.0, None, &dir.join("overview-private-remaining.pam"));
        private.tab = codex;
        render(&private, 320.0, None, &dir.join("codex-private-remaining.pam"));
    }

    /// Sends a left click at the centre of `element`, laid out at `width`, and
    /// returns what it published - the same path a real click takes.
    fn click(element: Element<'_, Message>, width: f32) -> Vec<Message> {
        use cosmic::iced::advanced::{Layout, Shell, clipboard};
        use cosmic::iced::{Event, mouse};

        let mut element = element;
        let mut tree = Tree::new(element.as_widget());
        let renderer = renderer();
        let limits = Limits::NONE.max_width(width).width(Length::Shrink);
        let node = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        let bounds = Rectangle::with_size(node.size());
        let cursor = mouse::Cursor::Available(bounds.center());
        let mut messages = Vec::new();
        for event in [
            mouse::Event::CursorMoved {
                position: bounds.center(),
            },
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            element.as_widget_mut().update(
                &mut tree,
                &Event::Mouse(event),
                Layout::new(&node),
                cursor,
                &renderer,
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &bounds,
            );
        }
        messages
    }

    fn codex_row(window: &Window, index: usize, overview: bool) -> AccountRow {
        let State::Loaded(payloads) = &window.state else {
            unreachable!()
        };
        AccountRow::new(&payloads[index], index, overview)
    }

    #[test]
    fn clicking_an_account_row_toggles_it() {
        let mut window = sample_window(Config::default());
        let row = codex_row(&window, 1, true);
        let messages = {
            let State::Loaded(payloads) = &window.state else {
                unreachable!()
            };
            click(
                window.account_row(&payloads[1], 1, row.clone()),
                narrowest_block(),
            )
        };
        assert!(
            matches!(messages.as_slice(), [Message::ToggleAccount(clicked)] if *clicked == row),
            "{messages:?}"
        );

        assert!(!window.is_open(&row, 1));
        for message in messages {
            let _ = window.update(message);
        }
        assert!(window.is_open(&row, 1));
        let _ = window.update(Message::ToggleAccount(row.clone()));
        assert!(!window.is_open(&row, 1));
    }

    /// Provider tabs open their first account and Overview starts collapsed,
    /// and opening an account in one view leaves the other view alone.
    #[test]
    fn account_rows_open_independently_per_view() {
        let mut window = sample_window(Config::default());
        let rows = [
            codex_row(&window, 0, true),
            codex_row(&window, 1, true),
            codex_row(&window, 0, false),
            codex_row(&window, 1, false),
        ];
        let open = |window: &Window| {
            [(0, 0), (1, 1), (2, 0), (3, 1)].map(|(row, index)| window.is_open(&rows[row], index))
        };
        assert_eq!(open(&window), [false, false, true, false]);

        let _ = window.update(Message::ToggleAccount(rows[1].clone()));
        assert_eq!(open(&window), [false, true, true, false]);
        let _ = window.update(Message::ToggleAccount(rows[2].clone()));
        assert_eq!(open(&window), [false, true, false, false]);
    }

    #[test]
    fn collapsed_rows_summarize_the_tightest_window() {
        let window = sample_window(Config::default());
        let State::Loaded(payloads) = &window.state else {
            unreachable!()
        };
        let personal = usage_windows(&payloads[0]);
        assert_eq!(tightest(&personal).and_then(|w| w.used_percent), Some(25.0));
        assert_eq!(
            window.collapsed_lines(&payloads[0], &personal, true),
            ["Session 3% · Weekly 25%"]
        );
        let tab = window.collapsed_lines(&payloads[0], &personal, false);
        assert_eq!(tab.len(), 2);
        assert!(tab[1].starts_with("Weekly 25% used · Resets in 4d"), "{tab:?}");

        // A window without a percentage is never the tightest, and an account
        // whose fetch failed says why instead of listing windows.
        let payloads = crate::codexbar::parse_usage_json(
            r#"[{"provider":"codex","usage":{
                "primary":{"windowMinutes":300},
                "secondary":{"usedPercent":40,"windowMinutes":10080}}},
            {"provider":"codex","error":{"message":"Token expired"}}]"#,
        )
        .unwrap();
        let windows = usage_windows(&payloads[0]);
        assert_eq!(tightest(&windows).and_then(|w| w.used_percent), Some(40.0));
        assert_eq!(
            window.collapsed_lines(&payloads[1], &usage_windows(&payloads[1]), true),
            ["Token expired"]
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
