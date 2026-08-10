//! Thin wrapper around the `codexbar` CLI shipped by
//! <https://github.com/steipete/CodexBar>.
//!
//! `codexbar usage --format json` prints a JSON **array** of `ProviderPayload`
//! objects (one per provider/account that was queried). The shape below mirrors
//! `Sources/CodexBarCLI/CLIPayloads.swift` and the documented example in
//! `docs/cli.md` of that repository:
//!
//! ```json
//! [
//!   {
//!     "provider": "codex",
//!     "account": "user@example.com",
//!     "version": "0.6.0",
//!     "source": "openai-web",
//!     "usage": {
//!       "primary":   { "usedPercent": 28, "windowMinutes": 300,   "resetsAt": "2025-12-04T19:15:00Z" },
//!       "secondary": { "usedPercent": 59, "windowMinutes": 10080, "resetsAt": "2025-12-05T17:00:00Z" },
//!       "tertiary": null,
//!       "updatedAt": "2025-12-04T18:10:22Z",
//!       "identity": { "providerID": "codex", "accountEmail": "user@example.com", "loginMethod": "plus" }
//!     },
//!     "credits": { "remaining": 112.4, "updatedAt": "2025-12-04T18:10:21Z" },
//!     "error": null
//!   }
//! ]
//! ```
//!
//! Swift's `JSONEncoder` is used with `.iso8601` date encoding, so every date is
//! an RFC 3339 string. Keys are the Swift property names, i.e. lowerCamelCase.
//!
//! Everything here is decoded defensively: every field is optional and unknown
//! fields are ignored, so a CodexBar release that adds or drops keys degrades
//! gracefully instead of blanking the applet. If a field name turns out to be
//! wrong, only the `serde` attributes below need adjusting.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// The CLI this applet drives. Every invocation is this name plus a fixed
/// argument list - inside a Flatpak, wrapped in `flatpak-spawn --host` - and
/// nothing user-supplied ever reaches a command line.
const PROGRAM: &str = "codexbar";

/// One entry of the `codexbar usage --format json` array.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPayload {
    /// Provider identifier, e.g. `codex`, `claude`, `copilot`.
    pub provider: String,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub usage: Option<UsageSnapshot>,
    #[serde(default)]
    pub credits: Option<CreditsSnapshot>,
    /// CodexBar's burn-rate projection, keyed by the same window names as
    /// `usage`. Absent for providers that cannot project (e.g. `antigravity`).
    #[serde(default)]
    pub pace: Option<PaceSnapshot>,
    #[serde(default)]
    pub error: Option<ProviderError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    /// Shortest rolling window (the "session" limit for Codex/Claude).
    #[serde(default)]
    pub primary: Option<RateLimitWindow>,
    /// Second window, normally the weekly limit.
    #[serde(default)]
    pub secondary: Option<RateLimitWindow>,
    /// Optional third window, normally monthly.
    #[serde(default)]
    pub tertiary: Option<RateLimitWindow>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub identity: Option<Identity>,
    #[serde(default)]
    pub codex_reset_credits: Option<CodexResetCredits>,
    /// Extra windows the provider reports beside the three numbered slots.
    /// Only their titles are read, never their numbers - see
    /// [`UsageSnapshot::window_label_overrides`]. Providers that report none
    /// simply have an empty list.
    #[serde(default)]
    pub extra_rate_windows: Vec<ExtraRateWindow>,
}

/// Who the numbers belong to. This, not the top-level `ProviderPayload::account`,
/// is where the live CLI reports the signed-in email.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Plan or login type, e.g. `plus`, `Antigravity Starter Quota`.
    #[serde(default)]
    pub login_method: Option<String>,
    /// Signed-in account, usually an email address. Absent for providers whose
    /// backing CLI does not report one (Claude, at the time of writing).
    #[serde(default)]
    pub account_email: Option<String>,
}

/// `usage.codexResetCredits`: OpenAI's "Limit Reset Credits", the periodic
/// grants that let a Codex user reset their weekly window early.
///
/// Undocumented in CodexBar's `docs/cli.md` and its CLI help, so this mirrors
/// `CodexRateLimitResetCredit` in CodexBar's Swift source and the live payload.
/// Only Codex populates it in practice, but nothing here assumes that - a
/// provider that never reports the field simply has none.
///
/// `availableCount` is deliberately not decoded: [`UsageSnapshot`] counts the
/// entries itself, the same way the macOS app does, rather than trusting a
/// total that could disagree with the array beside it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexResetCredits {
    #[serde(default)]
    pub credits: Vec<ResetCredit>,
}

/// One reset credit. Redeeming a credit resets the whole window, so each entry
/// is a discrete grant rather than a point balance.
///
/// Unlike the rest of this payload, these keys are snake_case (`expires_at`,
/// not `expiresAt`), so the field names map straight across and this is the one
/// struct in the file without a `rename_all` attribute.
#[derive(Debug, Clone, Deserialize)]
pub struct ResetCredit {
    /// `available`, `redeeming`, `redeemed` or `expired`. Any other value is
    /// treated as not redeemable.
    #[serde(default)]
    pub status: Option<String>,
    /// Absent for a credit that never expires.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub window_minutes: Option<u64>,
    #[serde(default)]
    pub resets_at: Option<DateTime<Utc>>,
    /// Human readable reset hint (e.g. "today at 3:00 PM") when the CLI has one.
    #[serde(default)]
    pub reset_description: Option<String>,
}

/// One entry of `usage.extraRateWindows`: a named window sitting outside the
/// numbered `primary`/`secondary`/`tertiary` slots.
///
/// These are not drawn as rows. Upstream notes that some of them carry reset
/// metadata without a real usage figure, so rendering their `usedPercent` would
/// invent an exhausted quota; the applet reads nothing from them but the title.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraRateWindow {
    /// Display name of the pool, e.g. "Gemini weekly".
    #[serde(default)]
    pub title: Option<String>,
    /// Stable identifier, e.g. `antigravity-quota-summary-gemini-weekly`.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub window: Option<RateLimitWindow>,
}

/// Top-level `pace` object, mirroring [`UsageSnapshot`]'s window keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaceSnapshot {
    #[serde(default)]
    pub primary: Option<PaceWindow>,
    #[serde(default)]
    pub secondary: Option<PaceWindow>,
    #[serde(default)]
    pub tertiary: Option<PaceWindow>,
}

/// Projection for one rate limit window. Every field is optional: the CLI omits
/// whichever parts it cannot compute (`etaSeconds` is missing when usage lasts
/// to the reset, for instance).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaceWindow {
    /// Burn rate relative to a linear budget, e.g. `onTrack`, `farAhead`.
    #[serde(default)]
    pub stage: Option<String>,
    /// Signed distance from the expected usage, in percentage points.
    #[serde(default)]
    pub delta_percent: Option<f64>,
    #[serde(default)]
    pub expected_used_percent: Option<f64>,
    #[serde(default)]
    pub will_last_to_reset: Option<bool>,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    /// Pre-rendered summary, e.g.
    /// "25% in deficit | Expected 50% used | Runs out in 1d 4h".
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditsSnapshot {
    #[serde(default)]
    pub remaining: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderError {
    pub message: String,
}

/// One entry of the `codexbar cost --format json --days 30` array.
///
/// The CLI only emits entries for providers that track cost locally (currently
/// Codex and Claude); others are simply absent rather than erroring, so a
/// missing entry means "no cost block for this provider".
///
/// `sessionCostUSD`/`sessionTokens` are the running totals for the current day,
/// which is what the macOS app labels "Today" / "Latest tokens".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostPayload {
    /// Provider identifier, matching [`ProviderPayload::provider`].
    pub provider: String,
    #[serde(default)]
    pub currency_code: Option<String>,
    #[serde(default, rename = "sessionCostUSD")]
    pub session_cost_usd: Option<f64>,
    #[serde(default)]
    pub session_tokens: Option<u64>,
    #[serde(default, rename = "last30DaysCostUSD")]
    pub last30_days_cost_usd: Option<f64>,
    #[serde(default)]
    pub last30_days_tokens: Option<u64>,
}

impl CostPayload {
    /// Whether any of the four displayed figures is present.
    pub fn has_figures(&self) -> bool {
        self.session_cost_usd.is_some()
            || self.session_tokens.is_some()
            || self.last30_days_cost_usd.is_some()
            || self.last30_days_tokens.is_some()
    }
}

/// Format a money amount the way the macOS app does, e.g. `$29.41`.
///
/// Only USD is ever reported today, so an unexpected `currencyCode` is appended
/// rather than guessed at.
pub fn format_cost(amount: f64, currency_code: Option<&str>) -> String {
    match currency_code {
        None | Some("USD") => format!("${amount:.2}"),
        Some(other) => format!("{amount:.2} {other}"),
    }
}

/// Abbreviate a token count, e.g. `19523312` becomes `19.5M`.
pub fn format_tokens(tokens: u64) -> String {
    const BILLION: f64 = 1_000_000_000.0;
    const MILLION: f64 = 1_000_000.0;
    const THOUSAND: f64 = 1_000.0;

    let value = tokens as f64;
    if value >= BILLION {
        format!("{}B", one_decimal(value / BILLION))
    } else if value >= MILLION {
        format!("{}M", one_decimal(value / MILLION))
    } else if value >= THOUSAND {
        format!("{}K", one_decimal(value / THOUSAND))
    } else {
        tokens.to_string()
    }
}

/// One decimal place with a trailing `.0` trimmed, so `26.0` reads as `26`.
fn one_decimal(value: f64) -> String {
    let text = format!("{value:.1}");
    match text.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => text,
    }
}

/// Replace the exotic spaces Swift's date formatting emits with plain ones.
///
/// Codex reports `resetDescription` as `Aug 10 at 11:39\u{202f}PM`; a narrow
/// no-break space is not in every UI font, and a missing glyph renders as no
/// gap at all rather than as a fallback space.
fn normalise_spaces(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

/// Render a duration in seconds as the CLI does, e.g. `1d 4h`, `3h 49m`, `12m`.
fn duration_text(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    if hours >= 24 {
        format!("{}d {}h", hours / 24, hours % 24)
    } else if hours > 0 {
        format!("{hours}h {}m", minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

impl ProviderPayload {
    /// The plan to show beside the snapshot age, where the provider reports one
    /// worth trusting.
    ///
    /// Claude's is dropped. On Linux its usage comes from driving the `claude`
    /// CLI in a pseudo-terminal and scraping the redrawing screen, and whatever
    /// followed "Login method:" is passed through unchanged when it matches no
    /// known plan. A capture caught mid-repaint therefore leaves a bare number
    /// that changes between refreshes - "21", then "25" - which reads as a
    /// fault in this applet rather than in the data it was handed.
    pub fn plan_label(&self) -> Option<String> {
        if self.provider.eq_ignore_ascii_case("claude") {
            return None;
        }
        self.usage.as_ref()?.plan_label()
    }

    /// Human readable provider name. CodexBar's JSON only carries the provider
    /// id, so the display label is derived here.
    pub fn label(&self) -> String {
        match self.provider.as_str() {
            "codex" => "Codex".to_string(),
            "claude" => "Claude".to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => "Unknown".to_string(),
                }
            }
        }
    }

    /// Signed-in account for this provider, usually an email address.
    ///
    /// The live CLI reports it as `usage.identity.accountEmail`; the top-level
    /// `account` documented in `docs/cli.md` is never populated in practice but
    /// is still honoured as a fallback in case some provider does emit it.
    pub fn account_text(&self) -> Option<&str> {
        self.usage
            .as_ref()
            .and_then(|usage| usage.identity.as_ref())
            .and_then(|identity| identity.account_email.as_deref())
            .or(self.account.as_deref())
    }
}

impl UsageSnapshot {
    /// Age of the snapshot, e.g. "Updated 1m ago". A snapshot dated in the
    /// future (clock skew) reads as "just now" rather than a negative age.
    pub fn updated_text(&self, now: DateTime<Utc>) -> Option<String> {
        let elapsed = now.signed_duration_since(self.updated_at?);
        let minutes = elapsed.num_minutes();
        Some(if minutes < 1 {
            "Updated just now".to_string()
        } else if minutes < 60 {
            format!("Updated {minutes}m ago")
        } else if elapsed.num_hours() < 24 {
            format!("Updated {}h ago", elapsed.num_hours())
        } else {
            format!("Updated {}d ago", elapsed.num_days())
        })
    }

    /// How many reset credits can actually be redeemed right now.
    pub fn available_reset_credits(&self, now: DateTime<Utc>) -> usize {
        match &self.codex_reset_credits {
            Some(reset) => reset
                .credits
                .iter()
                .filter(|credit| credit.is_available(now))
                .count(),
            None => 0,
        }
    }

    /// Reset-credit caption, e.g. "Limit reset credits: 2 available, soonest
    /// expires in 3d". `None` when there is nothing redeemable, which is the
    /// usual case even for Codex.
    pub fn reset_credits_text(&self, now: DateTime<Utc>) -> Option<String> {
        let available = self.available_reset_credits(now);
        if available == 0 {
            return None;
        }
        let mut text = format!("Limit reset credits: {available} available");
        if let Some(expiry) = self.soonest_reset_credit_expiry(now) {
            let seconds = expiry.signed_duration_since(now).num_seconds().max(0) as u64;
            text.push_str(&format!(", soonest expires in {}", duration_text(seconds)));
        }
        Some(text)
    }

    /// When the first redeemable credit lapses. `None` when none of them expire.
    fn soonest_reset_credit_expiry(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.codex_reset_credits
            .as_ref()?
            .credits
            .iter()
            .filter(|credit| credit.is_available(now))
            .filter_map(|credit| credit.expires_at)
            .min()
    }

    /// Replacement labels for the primary/secondary/tertiary slots, in that
    /// order, for the case where the derived labels would not tell two windows
    /// apart. `None` in a slot means [`RateLimitWindow::window_label`] stands.
    ///
    /// Antigravity is the reason this exists. It reports two separate quota
    /// pools - Gemini, and Claude/GPT - as `primary` and `secondary`, and both
    /// are 10080-minute windows, so both derive "Weekly" and the popup shows two
    /// rows nothing distinguishes. The CLI does name the pools, but only in
    /// `extraRateWindows`, whose entries line up positionally with the numbered
    /// slots: the first extra describes `primary`, the second `secondary`.
    ///
    /// Only titles are borrowed, never numbers, and the extras are never drawn
    /// as rows of their own.
    ///
    /// A slot is overridden only when all of the following hold, which leaves
    /// every provider whose labels already differ - Codex and Claude report a
    /// 300-minute session beside a weekly window - completely untouched:
    ///
    /// * two or more of the reported windows derive the same label, so there is
    ///   an actual collision to resolve;
    /// * the extra at the slot's own position exists and carries a title;
    /// * that extra's window has the same length and reset time as the slot's,
    ///   a shape check so an unrelated extra cannot capture a label by sitting
    ///   in the right position.
    ///
    /// `fallbacks` are the labels the caller uses for windows that report no
    /// `windowMinutes`, passed in so the collision is judged against the text
    /// that would actually be shown.
    pub fn window_label_overrides(&self, fallbacks: [&str; 3]) -> [Option<String>; 3] {
        let slots = [
            self.primary.as_ref(),
            self.secondary.as_ref(),
            self.tertiary.as_ref(),
        ];
        let labels: Vec<String> = slots
            .iter()
            .copied()
            .zip(fallbacks)
            .filter_map(|(window, fallback)| Some(window?.window_label(fallback)))
            .collect();
        let collides = labels
            .iter()
            .any(|label| labels.iter().filter(|other| *other == label).count() > 1);
        if !collides {
            return [None, None, None];
        }

        std::array::from_fn(|slot| {
            let window = slots[slot]?;
            let extra = self.extra_rate_windows.get(slot)?;
            let title = extra.title.as_deref()?.trim();
            let extra_window = extra.window.as_ref()?;
            if title.is_empty()
                || extra_window.window_minutes != window.window_minutes
                || extra_window.resets_at != window.resets_at
            {
                return None;
            }
            Some(title.to_string())
        })
    }

    /// Plan label from `identity.loginMethod`, capitalised, e.g. "Plus".
    pub fn plan_label(&self) -> Option<String> {
        let method = self.identity.as_ref()?.login_method.as_deref()?.trim();
        let mut chars = method.chars();
        let first = chars.next()?;
        Some(first.to_uppercase().collect::<String>() + chars.as_str())
    }
}

impl ResetCredit {
    /// Whether this credit can be redeemed now.
    ///
    /// Mirrors the macOS app: a credit counts only when its status is
    /// `available` *and* it has not lapsed. The payload's own `availableCount`
    /// is not consulted, so a stale or differently-counted total cannot put a
    /// number in the popup that the credits beside it do not support.
    fn is_available(&self, now: DateTime<Utc>) -> bool {
        self.status.as_deref() == Some("available")
            && self.expires_at.is_none_or(|expires| expires > now)
    }
}

impl RateLimitWindow {
    /// Fraction in `0.0..=1.0`, suitable for a progress bar.
    pub fn fraction(&self) -> f32 {
        (self.used_percent.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32
    }

    /// Label derived from the rolling window length.
    pub fn window_label(&self, fallback: &str) -> String {
        match self.window_minutes {
            Some(m) if m <= 300 => "Session".to_string(),
            Some(10080) => "Weekly".to_string(),
            Some(43200) => "Monthly".to_string(),
            Some(m) if m % 1440 == 0 => format!("{}-day", m / 1440),
            Some(m) if m % 60 == 0 => format!("{}-hour", m / 60),
            Some(m) => format!("{m}-minute"),
            None => fallback.to_string(),
        }
    }

    /// Reset text, e.g. "Resets in 4h 1m".
    ///
    /// A countdown computed from `resetsAt` is preferred over the CLI's
    /// `resetDescription`: the description is a localised wall-clock string
    /// ("Resets 3:50pm (Asia/Tokyo)") that is both longer than the popup's
    /// value column can hold and less useful than "how long have I got". The
    /// description is only used when there is no `resetsAt` to count down to,
    /// and its parenthesised timezone suffix is dropped for the same reason -
    /// it names the reader's own timezone, so it carries no information.
    pub fn reset_text(&self, now: DateTime<Utc>) -> Option<String> {
        if let Some(resets_at) = self.resets_at {
            let remaining = resets_at.signed_duration_since(now);
            if remaining.num_seconds() <= 0 {
                return Some("Resetting now".to_string());
            }
            return Some(format!(
                "Resets in {}",
                duration_text(remaining.num_seconds() as u64)
            ));
        }

        let description = normalise_spaces(self.reset_description.as_deref()?);
        let description = strip_parenthetical(description.trim());
        if description.is_empty() {
            return None;
        }
        if description.to_lowercase().starts_with("reset") {
            return Some(description.to_string());
        }
        Some(format!("Resets {description}"))
    }
}

/// Drop a trailing parenthesised group, e.g. the "(Asia/Tokyo)" CodexBar
/// appends to Claude's reset descriptions.
fn strip_parenthetical(text: &str) -> &str {
    match text.rfind('(') {
        Some(open) if text.ends_with(')') => text[..open].trim_end(),
        _ => text,
    }
}

impl PaceWindow {
    /// The CLI's `summary` split into its pipe-separated clauses, so a narrow
    /// popup can stack them instead of wrapping one long line.
    pub fn summary_lines(&self) -> Vec<String> {
        let Some(summary) = &self.summary else {
            return Vec::new();
        };
        summary
            .split('|')
            .map(str::trim)
            .filter(|clause| !clause.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The headline clause of `summary`, e.g. "On pace", "31% in reserve".
    pub fn stage_text(&self) -> Option<String> {
        self.summary_lines().into_iter().next()
    }

    /// How long the quota is projected to last, e.g. "Lasts until reset" or
    /// "Projected empty in 3h 49m".
    pub fn projection_text(&self) -> Option<String> {
        if self.will_last_to_reset == Some(true) {
            return Some("Lasts until reset".to_string());
        }
        Some(format!(
            "Projected empty in {}",
            duration_text(self.eta_seconds?)
        ))
    }
}

/// Parse the stdout of `codexbar usage --format json`.
///
/// Leading non-JSON chatter (warnings, progress output) is skipped by starting
/// at the first `[`, because the CLI only guarantees the array on stdout when
/// `--json-only` is in effect.
pub fn parse_usage_json(stdout: &str) -> Result<Vec<ProviderPayload>, String> {
    let start = stdout
        .find('[')
        .ok_or_else(|| "no JSON array in codexbar output".to_string())?;
    serde_json::from_str(&stdout[start..]).map_err(|e| format!("could not parse codexbar JSON: {e}"))
}

/// Run `codexbar usage --format json` and parse its output.
///
/// The CLI exits non-zero when an individual provider fails but still prints a
/// payload carrying the `error` field, so a parseable stdout always wins over
/// the exit status.
pub async fn fetch_usage() -> Result<Vec<ProviderPayload>, String> {
    let output = run_codexbar(&["usage", "--format", "json"]).await?;
    parse_output(&output, parse_usage_json)
}

/// Parse the stdout of `codexbar cost --format json --days 30`.
pub fn parse_cost_json(stdout: &str) -> Result<Vec<CostPayload>, String> {
    let start = stdout
        .find('[')
        .ok_or_else(|| "no JSON array in codexbar output".to_string())?;
    serde_json::from_str(&stdout[start..]).map_err(|e| format!("could not parse codexbar JSON: {e}"))
}

/// Run `codexbar cost --format json --days 30` and parse its output.
///
/// `--refresh` is deliberately not passed: the cached read takes well under a
/// second, which is what makes this safe to call on the same 60s tick as
/// [`fetch_usage`].
pub async fn fetch_cost() -> Result<Vec<CostPayload>, String> {
    let output = run_codexbar(&["cost", "--format", "json", "--days", "30"]).await?;
    parse_output(&output, parse_cost_json)
}

/// Turn a completed CLI run into parsed payloads.
///
/// The CLI exits non-zero when an individual provider fails but still prints a
/// payload carrying the `error` field, so a parseable stdout always wins over
/// the exit status. When stdout is unusable, stderr is the more helpful message.
fn parse_output<T>(
    output: &std::process::Output,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse(&stdout) {
        Ok(parsed) => Ok(parsed),
        Err(parse_error) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            if stderr.is_empty() {
                Err(parse_error)
            } else {
                Err(stderr.to_string())
            }
        }
    }
}

/// Run the `codexbar` CLI, trying each install location in turn.
///
/// `PATH` always comes first, so a distro package or any binary the user has
/// put on their `PATH` wins; everything after it is a pure fallback, tried only
/// when the previous candidate does not exist.
///
/// Inside a Flatpak "does not exist" cannot be read off the exit status:
/// `flatpak-spawn` exits 1 both when the host binary is missing and when the
/// binary ran and failed, so each candidate is resolved on the host first.
async fn run_codexbar(args: &[&str]) -> Result<std::process::Output, String> {
    // cosmic-panel applets are launched by the systemd graphical session, which
    // does not source ~/.bashrc or ~/.profile, so managers that only extend PATH
    // there (Homebrew's `shellenv`, ~/.local/bin added by some installers) are
    // invisible here even though `codexbar` works fine in a terminal.
    let mut candidates = vec![PathBuf::from(PROGRAM)];
    candidates.extend(fallback_candidates());

    // In a Flatpak build the CLI lives on the host, not in the sandbox, and has
    // to stay there: it reads the user's provider credentials from ~/.codex,
    // ~/.claude and friends, so a bundled sandboxed copy would need filesystem
    // permissions broad enough to defeat the point of sandboxing it. Each
    // invocation is therefore handed to `flatpak-spawn --host`. The candidate
    // paths need no adjusting - $HOME is not rewritten inside the sandbox, so
    // they already name host locations.
    let sandboxed = in_flatpak();

    for candidate in &candidates {
        // A missing host binary does not surface as `NotFound` here - that would
        // describe `flatpak-spawn`, not what it was asked to run - and its exit
        // status is 1 whether the binary was missing or ran and failed. Ask the
        // host to resolve the candidate first, so the fallback chain has
        // something unambiguous to advance on.
        if sandboxed && !resolves_on_host(candidate).await {
            continue;
        }
        match run_cli(candidate, args, sandboxed).await {
            Ok(output) => return Ok(output),
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("could not run codexbar: {e}")),
        }
    }
    if sandboxed {
        Err("codexbar CLI not found on the host's PATH, in ~/.local/bin, or in Homebrew's \
             bin dir.\nThe applet is sandboxed and runs it on the host, so install it there \
             from github.com/steipete/CodexBar."
            .to_string())
    } else {
        Err("codexbar CLI not found on PATH, in ~/.local/bin, or in Homebrew's bin dir.\n\
             Install it from github.com/steipete/CodexBar."
            .to_string())
    }
}

/// Whether the applet is running inside a Flatpak sandbox.
///
/// `/.flatpak-info` is placed in every sandbox by flatpak itself. `$FLATPAK_ID`
/// is not a reliable substitute: it is only exported for launches that go
/// through the desktop file, and cosmic-panel spawns applets directly.
pub(crate) fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

async fn run_cli(
    program: &Path,
    args: &[&str],
    sandboxed: bool,
) -> std::io::Result<std::process::Output> {
    let mut command = if sandboxed {
        let mut command = tokio::process::Command::new("flatpak-spawn");
        command.arg("--host").arg(program);
        command
    } else {
        tokio::process::Command::new(program)
    };
    command.args(args).stdin(Stdio::null()).output().await
}

/// Whether the host can run `candidate`, asked before spawning it for real.
///
/// `command -v` handles both shapes of the candidate list: a bare name is looked
/// up on the host's `PATH`, an absolute path resolves when it is there. Whether
/// a non-executable file at that path counts is left to the host's `/bin/sh` -
/// bash rejects it, dash does not - which at worst costs one failed spawn
/// before the error is reported.
///
/// The portal runs the command in the session's environment, not the sandbox's:
/// `PATH` there is the login shell's, so the bare name resolves against
/// `~/.local/bin` and Homebrew even though the sandbox's own `PATH` is only
/// `/app/bin:/usr/bin`. The absolute candidates still matter for sessions that
/// never put those directories on `PATH` at all.
///
/// A failure to spawn the probe counts as unresolved and moves the loop on. That
/// conflates an unresolvable candidate with the portal being unavailable, so a
/// broken portal reports the CLI as missing rather than as unreachable.
async fn resolves_on_host(candidate: &Path) -> bool {
    tokio::process::Command::new("flatpak-spawn")
        .arg("--host")
        .arg("/bin/sh")
        .arg("-c")
        .arg(r#"command -v "$1""#)
        .arg("sh")
        .arg(candidate)
        .stdin(Stdio::null())
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

/// Where to look when `codexbar` is not on `PATH`.
///
/// `dirs` resolves `~/.local/bin` through `$XDG_BIN_HOME` where it is set, and
/// Homebrew on Linux installs either to the shared `/home/linuxbrew/.linuxbrew`
/// prefix or to a per-user `~/.linuxbrew` when the shared one is not writable.
/// Its `bin/codexbar` is a wrapper script that execs the real binary under
/// `Cellar/codexbar/<version>/`, so following `bin/codexbar` is enough.
fn fallback_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(3);
    if let Some(bin) = dirs::executable_dir() {
        candidates.push(bin.join(PROGRAM));
    }
    candidates.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin").join(PROGRAM));
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".linuxbrew/bin").join(PROGRAM));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two providers, mirroring the documented `docs/cli.md` payload shape.
    const MULTI_PROVIDER: &str = r#"[
      {
        "provider": "codex",
        "account": "user@example.com",
        "version": "0.6.0",
        "source": "openai-web",
        "status": {
          "indicator": "none",
          "description": "Operational",
          "updatedAt": "2025-12-04T17:55:00Z",
          "url": "https://status.openai.com/"
        },
        "usage": {
          "primary": {
            "usedPercent": 28,
            "windowMinutes": 300,
            "resetsAt": "2025-12-04T19:15:00Z"
          },
          "secondary": {
            "usedPercent": 59,
            "windowMinutes": 10080,
            "resetsAt": "2025-12-05T17:00:00Z"
          },
          "tertiary": null,
          "updatedAt": "2025-12-04T18:10:22Z",
          "identity": {
            "providerID": "codex",
            "accountEmail": "user@example.com",
            "accountOrganization": null,
            "loginMethod": "plus"
          }
        },
        "pace": {
          "primary": {
            "stage": "ahead",
            "deltaPercent": 12,
            "expectedUsedPercent": 16,
            "willLastToReset": false,
            "etaSeconds": 9000,
            "summary": "12% in deficit"
          }
        },
        "credits": { "remaining": 112.4, "updatedAt": "2025-12-04T18:10:21Z" }
      },
      {
        "provider": "claude",
        "account": null,
        "version": null,
        "source": "claude-cli",
        "usage": {
          "primary": {
            "usedPercent": 4,
            "windowMinutes": 300,
            "resetsAt": null,
            "resetDescription": "today at 3:00 PM"
          },
          "secondary": null,
          "tertiary": null,
          "updatedAt": "2025-12-04T18:10:22Z"
        }
      }
    ]"#;

    /// No providers configured/enabled: the CLI still emits a valid array.
    const EMPTY: &str = "[]";

    /// Real `codexbar usage --format json` output from CodexBar 0.47.0
    /// (installed via Homebrew, `codexbar --version` reports `CodexBar 0.47.0`),
    /// with account emails redacted. Confirms the parser handles the live
    /// schema, which differs from `docs/cli.md` in a few ways not covered by
    /// `MULTI_PROVIDER` above: extra top-level `pace` objects, a `credits`
    /// object with an additional `events` array, `usage.primary: null`
    /// alongside a populated `secondary`, and an `antigravity` provider whose
    /// windows carry no `windowMinutes` at all.
    const REAL_WORLD: &str = r#"[{"pace": {"secondary": {"expectedUsedPercent": 50, "deltaPercent": 25, "etaSeconds": 100804, "stage": "farAhead", "willLastToReset": false, "summary": "25% in deficit | Expected 50% used | Runs out in 1d 4h"}}, "source": "oauth", "provider": "codex", "version": "0.146.1", "credits": {"events": [], "updatedAt": "2026-08-07T02:40:08Z", "remaining": 0}, "usage": {"identity": {"loginMethod": "plus", "accountEmail": "redacted@example.com", "providerID": "codex"}, "updatedAt": "2026-08-07T02:40:08Z", "primary": null, "codexResetCredits": {"credits": [], "updatedAt": "2026-08-07T02:40:08Z", "availableCount": 0}, "secondary": {"resetDescription": "Aug 10 at 11:39 PM", "usedPercent": 75, "windowMinutes": 10080, "resetsAt": "2026-08-10T14:39:58Z"}, "tertiary": null, "dataConfidence": "exact", "loginMethod": "plus", "accountEmail": "redacted@example.com"}}, {"pace": {"secondary": {"willLastToReset": true, "deltaPercent": -31, "expectedUsedPercent": 49, "stage": "farBehind", "summary": "31% in reserve | Expected 49% used | Lasts until reset"}, "primary": {"willLastToReset": false, "deltaPercent": 1, "expectedUsedPercent": 17, "stage": "onTrack", "etaSeconds": 13787, "summary": "On pace | Expected 17% used | Projected empty in 3h 50m"}}, "source": "claude", "provider": "claude", "usage": {"identity": {"providerID": "claude"}, "tertiary": null, "secondary": {"usedPercent": 18, "windowMinutes": 10080, "resetsAt": "2026-08-10T16:00:00Z", "resetDescription": "Resets Aug 11, 1am (Asia/Tokyo)"}, "updatedAt": "2026-08-07T02:40:26Z", "primary": {"usedPercent": 18, "windowMinutes": 300, "resetsAt": "2026-08-07T06:50:00Z", "resetDescription": "Resets 3:50pm (Asia/Tokyo)"}}}, {"source": "cli", "provider": "antigravity", "usage": {"identity": {"accountEmail": "redacted2@example.com", "providerID": "antigravity", "loginMethod": "Antigravity Starter Quota"}, "updatedAt": "2026-08-07T02:40:29Z", "primary": {"usedPercent": 0, "resetsAt": "2026-08-14T02:40:28Z"}, "tertiary": null, "secondary": {"usedPercent": 0, "resetsAt": "2026-08-14T02:40:28Z"}, "loginMethod": "Antigravity Starter Quota", "accountEmail": "redacted2@example.com"}}]"#;

    /// A provider that failed: `usage` is null, `error` carries the reason.
    const ERRORED: &str = r#"[
      {
        "provider": "codex",
        "account": null,
        "version": null,
        "source": "openai-web",
        "usage": null,
        "credits": null,
        "error": { "code": 3, "message": "Not signed in", "kind": "auth" }
      }
    ]"#;

    /// Real `codexbar cost --format json --days 30` output, trimmed of the
    /// `totals`/`daily`/`projects` members this applet does not decode. Only
    /// providers that track cost locally appear at all, which is why there is
    /// no `antigravity` entry here even though `usage` reports one.
    const COST: &str = r#"[
      {
        "provider": "codex",
        "source": "local",
        "currencyCode": "USD",
        "historyDays": 30,
        "updatedAt": "2026-08-07T03:39:09Z",
        "sessionCostUSD": 0,
        "sessionTokens": 0,
        "last30DaysCostUSD": 362.66142439,
        "last30DaysTokens": 557826793,
        "projects": []
      },
      {
        "provider": "claude",
        "source": "local",
        "currencyCode": "USD",
        "historyDays": 30,
        "updatedAt": "2026-08-07T03:38:57Z",
        "sessionCostUSD": 11.125085000000006,
        "sessionTokens": 19523312,
        "last30DaysCostUSD": 327.4576672,
        "last30DaysTokens": 340952636,
        "projects": []
      }
    ]"#;

    #[test]
    fn parses_empty_payload() {
        assert!(parse_usage_json(EMPTY).unwrap().is_empty());
    }

    #[test]
    fn parses_multiple_providers() {
        let payloads = parse_usage_json(MULTI_PROVIDER).unwrap();
        assert_eq!(payloads.len(), 2);

        let codex = &payloads[0];
        assert_eq!(codex.label(), "Codex");
        assert_eq!(codex.account.as_deref(), Some("user@example.com"));
        assert_eq!(codex.credits.as_ref().unwrap().remaining, Some(112.4));

        let usage = codex.usage.as_ref().unwrap();
        let primary = usage.primary.as_ref().unwrap();
        assert_eq!(primary.used_percent, Some(28.0));
        assert_eq!(primary.window_label("Primary"), "Session");
        assert!((primary.fraction() - 0.28).abs() < 1e-6);

        let secondary = usage.secondary.as_ref().unwrap();
        assert_eq!(secondary.window_label("Secondary"), "Weekly");
        assert!(usage.tertiary.is_none());

        let claude = &payloads[1];
        assert_eq!(claude.label(), "Claude");
        assert!(claude.credits.is_none());
        let claude_primary = claude
            .usage
            .as_ref()
            .unwrap()
            .primary
            .as_ref()
            .unwrap();
        assert_eq!(
            claude_primary.reset_text(Utc::now()).as_deref(),
            Some("Resets today at 3:00 PM")
        );
    }

    #[test]
    fn parses_real_world_payload() {
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        assert_eq!(payloads.len(), 3);

        let codex = &payloads[0];
        assert_eq!(codex.label(), "Codex");
        assert_eq!(codex.credits.as_ref().unwrap().remaining, Some(0.0));
        let usage = codex.usage.as_ref().unwrap();
        assert!(usage.primary.is_none());
        let secondary = usage.secondary.as_ref().unwrap();
        assert_eq!(secondary.used_percent, Some(75.0));
        assert_eq!(secondary.window_label("Secondary"), "Weekly");
        // `resetsAt` is present, so the countdown wins over the description.
        let before_reset: DateTime<Utc> = "2026-08-07T14:39:58Z".parse().unwrap();
        assert_eq!(
            secondary.reset_text(before_reset).as_deref(),
            Some("Resets in 3d 0h")
        );

        let claude = &payloads[1];
        assert_eq!(claude.label(), "Claude");
        let claude_usage = claude.usage.as_ref().unwrap();
        assert_eq!(
            claude_usage.primary.as_ref().unwrap().window_label("P"),
            "Session"
        );
        // Claude reports both, and the countdown is preferred: the description
        // is a localised wall-clock string too wide for the popup's value cell.
        let before_reset: DateTime<Utc> = "2026-08-07T05:50:00Z".parse().unwrap();
        assert_eq!(
            claude_usage
                .primary
                .as_ref()
                .unwrap()
                .reset_text(before_reset)
                .as_deref(),
            Some("Resets in 1h 0m")
        );

        // antigravity's windows carry no `windowMinutes` at all, only usedPercent/resetsAt.
        let antigravity = &payloads[2];
        assert_eq!(antigravity.label(), "Antigravity");
        let ag_usage = antigravity.usage.as_ref().unwrap();
        let ag_primary = ag_usage.primary.as_ref().unwrap();
        assert_eq!(ag_primary.used_percent, Some(0.0));
        assert!(ag_primary.window_minutes.is_none());
        assert_eq!(ag_primary.window_label("Primary"), "Primary");
    }

    /// The live CLI carries the email in `usage.identity.accountEmail`, never in
    /// the top-level `account` that `docs/cli.md` documents.
    #[test]
    fn resolves_the_account_from_identity() {
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        assert!(payloads.iter().all(|payload| payload.account.is_none()));

        assert_eq!(payloads[0].account_text(), Some("redacted@example.com"));
        assert_eq!(payloads[2].account_text(), Some("redacted2@example.com"));
        // Claude's identity reports neither loginMethod nor accountEmail.
        assert_eq!(payloads[1].account_text(), None);

        // The documented top-level field still wins when there is no identity.
        let documented = parse_usage_json(MULTI_PROVIDER).unwrap();
        assert_eq!(documented[1].account_text(), None);
        assert_eq!(
            parse_usage_json(r#"[{"provider": "codex", "account": "legacy@example.com"}]"#)
                .unwrap()[0]
                .account_text(),
            Some("legacy@example.com")
        );
    }

    #[test]
    fn parses_provider_error() {
        let payloads = parse_usage_json(ERRORED).unwrap();
        assert!(payloads[0].usage.is_none());
        assert_eq!(payloads[0].error.as_ref().unwrap().message, "Not signed in");
    }

    #[test]
    fn skips_leading_chatter() {
        let payloads = parse_usage_json("warning: cache miss\n[]").unwrap();
        assert!(payloads.is_empty());
    }

    #[test]
    fn reports_unparseable_output() {
        assert!(parse_usage_json("command not found").is_err());
        assert!(parse_usage_json("[{").is_err());
    }

    #[test]
    fn formats_countdown_from_resets_at() {
        let now: DateTime<Utc> = "2025-12-04T17:15:00Z".parse().unwrap();
        let payloads = parse_usage_json(MULTI_PROVIDER).unwrap();
        let primary = payloads[0]
            .usage
            .as_ref()
            .unwrap()
            .primary
            .as_ref()
            .unwrap();
        assert_eq!(primary.reset_text(now).as_deref(), Some("Resets in 2h 0m"));

        let secondary = payloads[0]
            .usage
            .as_ref()
            .unwrap()
            .secondary
            .as_ref()
            .unwrap();
        assert_eq!(
            secondary.reset_text(now).as_deref(),
            Some("Resets in 23h 45m")
        );
    }

    #[test]
    fn parses_pace_from_documented_shape() {
        let payloads = parse_usage_json(MULTI_PROVIDER).unwrap();
        let pace = payloads[0].pace.as_ref().unwrap();
        let primary = pace.primary.as_ref().unwrap();
        assert_eq!(primary.stage.as_deref(), Some("ahead"));
        assert_eq!(primary.delta_percent, Some(12.0));
        assert_eq!(primary.expected_used_percent, Some(16.0));
        assert_eq!(primary.will_last_to_reset, Some(false));
        assert_eq!(primary.eta_seconds, Some(9000));
        assert_eq!(primary.summary_lines(), vec!["12% in deficit".to_string()]);
        assert!(pace.secondary.is_none());

        // The second provider has no `pace` object at all.
        assert!(payloads[1].pace.is_none());
    }

    #[test]
    fn parses_pace_from_real_world_payload() {
        let payloads = parse_usage_json(REAL_WORLD).unwrap();

        let codex = payloads[0].pace.as_ref().unwrap();
        assert!(codex.primary.is_none());
        let codex_secondary = codex.secondary.as_ref().unwrap();
        assert_eq!(codex_secondary.stage.as_deref(), Some("farAhead"));
        assert_eq!(codex_secondary.delta_percent, Some(25.0));
        assert_eq!(codex_secondary.eta_seconds, Some(100804));
        assert_eq!(
            codex_secondary.summary_lines(),
            vec![
                "25% in deficit".to_string(),
                "Expected 50% used".to_string(),
                "Runs out in 1d 4h".to_string(),
            ]
        );

        let claude = payloads[1].pace.as_ref().unwrap();
        let claude_primary = claude.primary.as_ref().unwrap();
        assert_eq!(claude_primary.stage.as_deref(), Some("onTrack"));
        assert_eq!(
            claude_primary.summary_lines(),
            vec![
                "On pace".to_string(),
                "Expected 17% used".to_string(),
                "Projected empty in 3h 50m".to_string(),
            ]
        );
        let claude_secondary = claude.secondary.as_ref().unwrap();
        assert_eq!(claude_secondary.delta_percent, Some(-31.0));
        assert_eq!(claude_secondary.will_last_to_reset, Some(true));
        assert_eq!(claude_secondary.eta_seconds, None);
        assert!(claude.tertiary.is_none());

        // antigravity reports usage but no projection at all.
        assert!(payloads[2].pace.is_none());
    }

    /// Codex really does report a narrow no-break space before the meridiem.
    const NARROW_SPACE: &str = r#"[
      {
        "provider": "codex",
        "usage": {
          "primary": { "resetDescription": "Aug 10 at 11:39\u202fPM" }
        }
      }
    ]"#;

    #[test]
    fn drops_the_timezone_suffix_from_a_description_fallback() {
        // No `resetsAt`, so the description is all there is - minus the
        // "(Asia/Tokyo)" that names the reader's own timezone.
        let payloads = parse_usage_json(
            r#"[{"provider": "claude", "usage": {"primary":
                 {"resetDescription": "Resets 3:50pm (Asia/Tokyo)"}}}]"#,
        )
        .unwrap();
        let window = payloads[0]
            .usage
            .as_ref()
            .unwrap()
            .primary
            .as_ref()
            .unwrap();
        assert_eq!(
            window.reset_text(Utc::now()).as_deref(),
            Some("Resets 3:50pm")
        );
    }

    #[test]
    fn normalises_exotic_spaces_in_reset_description() {
        let payloads = parse_usage_json(NARROW_SPACE).unwrap();
        let window = payloads[0]
            .usage
            .as_ref()
            .unwrap()
            .primary
            .as_ref()
            .unwrap();

        let text = window.reset_text(Utc::now()).unwrap();
        assert_eq!(text, "Resets Aug 10 at 11:39 PM");
        assert!(text.chars().all(|c| !c.is_whitespace() || c == ' '));
    }

    /// `usage.codexResetCredits` exactly as the live CLI emits it when the
    /// account holds no reset credits.
    const RESET_CREDITS_EMPTY: &str = r#"[
      {
        "provider": "codex",
        "usage": {
          "codexResetCredits": {
            "credits": [],
            "availableCount": 0,
            "updatedAt": "2026-08-07T11:51:21Z"
          }
        }
      }
    ]"#;

    /// A populated grant list. `availableCount` deliberately disagrees with the
    /// array so the test proves the filter, not the number: of the four
    /// credits only the first is redeemable - the others are expired by status,
    /// already redeemed, and expired by date despite still saying "available".
    const RESET_CREDITS_POPULATED: &str = r#"[
      {
        "provider": "codex",
        "usage": {
          "codexResetCredits": {
            "availableCount": 4,
            "credits": [
              {
                "id": "a",
                "reset_type": "weekly",
                "status": "available",
                "granted_at": "2026-08-01T00:00:00Z",
                "expires_at": "2026-08-10T00:00:00Z",
                "title": "Limit reset",
                "description": null
              },
              {
                "id": "b",
                "reset_type": "weekly",
                "status": "expired",
                "granted_at": "2026-07-01T00:00:00Z",
                "expires_at": "2026-07-15T00:00:00Z"
              },
              {
                "id": "c",
                "reset_type": "weekly",
                "status": "redeemed",
                "granted_at": "2026-07-20T00:00:00Z",
                "expires_at": null,
                "redeemed_at": "2026-07-21T00:00:00Z"
              },
              {
                "id": "d",
                "reset_type": "weekly",
                "status": "available",
                "granted_at": "2026-07-01T00:00:00Z",
                "expires_at": "2026-07-02T00:00:00Z"
              }
            ]
          }
        }
      }
    ]"#;

    /// Two redeemable credits, one of which never expires.
    const RESET_CREDITS_NO_EXPIRY: &str = r#"[
      {
        "provider": "codex",
        "usage": {
          "codexResetCredits": {
            "credits": [
              { "id": "a", "status": "available", "expires_at": null },
              { "id": "b", "status": "available", "expires_at": null }
            ]
          }
        }
      }
    ]"#;

    fn usage_of(payload: &str) -> UsageSnapshot {
        parse_usage_json(payload).unwrap()[0]
            .usage
            .clone()
            .unwrap()
    }

    #[test]
    fn empty_reset_credits_show_nothing() {
        let usage = usage_of(RESET_CREDITS_EMPTY);
        assert!(usage.codex_reset_credits.is_some());
        assert_eq!(usage.available_reset_credits(Utc::now()), 0);
        assert_eq!(usage.reset_credits_text(Utc::now()), None);
    }

    #[test]
    fn counts_only_redeemable_reset_credits() {
        let now: DateTime<Utc> = "2026-08-07T00:00:00Z".parse().unwrap();
        let usage = usage_of(RESET_CREDITS_POPULATED);

        // Four entries and an availableCount of 4, but only one is redeemable.
        assert_eq!(usage.codex_reset_credits.as_ref().unwrap().credits.len(), 4);
        assert_eq!(usage.available_reset_credits(now), 1);
        assert_eq!(
            usage.reset_credits_text(now).as_deref(),
            Some("Limit reset credits: 1 available, soonest expires in 3d 0h")
        );

        // Past the last expiry, nothing is redeemable.
        let later: DateTime<Utc> = "2026-08-11T00:00:00Z".parse().unwrap();
        assert_eq!(usage.available_reset_credits(later), 0);
        assert_eq!(usage.reset_credits_text(later), None);
    }

    #[test]
    fn reset_credits_without_expiry_omit_the_expiry_clause() {
        let usage = usage_of(RESET_CREDITS_NO_EXPIRY);
        let now = Utc::now();
        assert_eq!(usage.available_reset_credits(now), 2);
        assert_eq!(
            usage.reset_credits_text(now).as_deref(),
            Some("Limit reset credits: 2 available")
        );
    }

    #[test]
    fn providers_without_reset_credits_report_none() {
        // The real-world payload has an empty `credits` array for Codex and no
        // `codexResetCredits` at all for Claude or antigravity.
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        for payload in &payloads {
            let usage = payload.usage.as_ref().unwrap();
            assert_eq!(usage.available_reset_credits(Utc::now()), 0);
            assert_eq!(usage.reset_credits_text(Utc::now()), None);
        }
        assert!(
            payloads[1]
                .usage
                .as_ref()
                .unwrap()
                .codex_reset_credits
                .is_none()
        );
    }

    /// Live `codexbar usage --format json` output for antigravity, captured
    /// from the CLI with the account email redacted. Its two windows are
    /// separate quota pools rather than a session and a weekly limit, but both
    /// are 10080-minute windows, so both derive "Weekly" and only
    /// `extraRateWindows` says which pool is which.
    const ANTIGRAVITY_EXTRAS: &str = r#"[
      {
        "provider": "antigravity",
        "usage": {
          "primary":   { "usedPercent": 0, "windowMinutes": 10080, "resetsAt": "2026-08-17T08:24:14Z" },
          "secondary": { "usedPercent": 0, "windowMinutes": 10080, "resetsAt": "2026-08-17T08:24:14Z" },
          "tertiary": null,
          "extraRateWindows": [
            { "title": "Gemini weekly",     "id": "antigravity-quota-summary-gemini-weekly",
              "window": { "windowMinutes": 10080, "usedPercent": 0, "resetsAt": "2026-08-17T08:24:14Z" } },
            { "title": "Claude/GPT weekly", "id": "antigravity-quota-summary-3p-weekly",
              "window": { "windowMinutes": 10080, "resetsAt": "2026-08-17T08:24:14Z", "usedPercent": 0 } }
          ],
          "identity": {
            "accountEmail": "redacted@example.com",
            "providerID": "antigravity",
            "loginMethod": "Antigravity Starter Quota"
          },
          "updatedAt": "2026-08-10T08:24:14Z"
        },
        "source": "cli"
      }
    ]"#;

    /// The same payload with the extras' reset times moved a day out, so they
    /// no longer match the windows they sit beside.
    const ANTIGRAVITY_MISMATCHED_EXTRAS: &str = r#"[
      {
        "provider": "antigravity",
        "usage": {
          "primary":   { "usedPercent": 0, "windowMinutes": 10080, "resetsAt": "2026-08-17T08:24:14Z" },
          "secondary": { "usedPercent": 0, "windowMinutes": 10080, "resetsAt": "2026-08-17T08:24:14Z" },
          "extraRateWindows": [
            { "title": "Gemini weekly",
              "window": { "windowMinutes": 10080, "usedPercent": 0, "resetsAt": "2026-08-18T08:24:14Z" } },
            { "title": "Claude/GPT weekly",
              "window": { "windowMinutes": 4320, "usedPercent": 0, "resetsAt": "2026-08-17T08:24:14Z" } }
          ]
        },
        "source": "cli"
      }
    ]"#;

    /// The labels the two tabs show, i.e. an override where there is one and
    /// the derived label everywhere else.
    fn labels_of(payload: &str) -> Vec<String> {
        let usage = usage_of(payload);
        let overrides = usage.window_label_overrides(["Session", "Weekly", "Monthly"]);
        [
            usage.primary.as_ref(),
            usage.secondary.as_ref(),
            usage.tertiary.as_ref(),
        ]
        .into_iter()
        .zip(overrides)
        .zip(["Session", "Weekly", "Monthly"])
        .filter_map(|((window, label), fallback)| {
            let window = window?;
            Some(label.unwrap_or_else(|| window.window_label(fallback)))
        })
        .collect()
    }

    #[test]
    fn names_antigravitys_colliding_windows_from_their_extras() {
        assert_eq!(
            labels_of(ANTIGRAVITY_EXTRAS),
            ["Gemini weekly", "Claude/GPT weekly"]
        );
    }

    #[test]
    fn leaves_providers_with_distinct_windows_alone() {
        // Claude reports a 300-minute session beside a weekly window, so there
        // is no collision and nothing to override.
        let claude = parse_usage_json(REAL_WORLD).unwrap()[1]
            .usage
            .clone()
            .unwrap();
        assert_eq!(
            claude.window_label_overrides(["Session", "Weekly", "Monthly"]),
            [None, None, None]
        );
    }

    #[test]
    fn ignores_extras_that_do_not_match_the_window_beside_them() {
        // Both extras are titled, and the labels do collide, but one differs in
        // reset time and the other in window length, so neither may claim a row.
        let usage = usage_of(ANTIGRAVITY_MISMATCHED_EXTRAS);
        assert_eq!(usage.extra_rate_windows.len(), 2);
        assert_eq!(labels_of(ANTIGRAVITY_MISMATCHED_EXTRAS), ["Weekly", "Weekly"]);
    }

    #[test]
    fn derives_labels_as_before_without_extras() {
        // Codex: a session and a weekly window, neither colliding. Antigravity
        // in the real-world payload reports no window lengths at all, so both
        // labels come from the callers' fallbacks and stay distinct.
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        for payload in &payloads {
            let usage = payload.usage.as_ref().unwrap();
            assert!(usage.extra_rate_windows.is_empty());
        }
        assert_eq!(labels_of(MULTI_PROVIDER), ["Session", "Weekly"]);
        assert_eq!(
            payloads[2]
                .usage
                .as_ref()
                .unwrap()
                .window_label_overrides(["Session", "Weekly", "Monthly"]),
            [None, None, None]
        );
    }

    #[test]
    fn parses_cost_payload() {
        let payloads = parse_cost_json(COST).unwrap();
        assert_eq!(payloads.len(), 2);

        let codex = &payloads[0];
        assert_eq!(codex.provider, "codex");
        assert_eq!(codex.currency_code.as_deref(), Some("USD"));
        assert_eq!(codex.session_cost_usd, Some(0.0));
        assert_eq!(codex.session_tokens, Some(0));
        assert_eq!(codex.last30_days_cost_usd, Some(362.66142439));
        assert_eq!(codex.last30_days_tokens, Some(557826793));
        assert!(codex.has_figures());

        let claude = &payloads[1];
        assert_eq!(claude.provider, "claude");
        assert_eq!(claude.session_tokens, Some(19523312));
        assert_eq!(claude.last30_days_tokens, Some(340952636));

        // Providers without local cost tracking are absent rather than errored.
        assert!(!payloads.iter().any(|c| c.provider == "antigravity"));
    }

    #[test]
    fn cost_payload_without_figures_is_detected() {
        let payloads = parse_cost_json(r#"[{"provider": "codex"}]"#).unwrap();
        assert!(!payloads[0].has_figures());
        assert!(parse_cost_json("[]").unwrap().is_empty());
        assert!(parse_cost_json("command not found").is_err());
    }

    #[test]
    fn formats_costs_and_token_counts() {
        assert_eq!(format_cost(0.0, Some("USD")), "$0.00");
        assert_eq!(format_cost(29.412, None), "$29.41");
        assert_eq!(format_cost(29.412, Some("EUR")), "29.41 EUR");

        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(340_952), "341K");
        assert_eq!(format_tokens(19_523_312), "19.5M");
        assert_eq!(format_tokens(26_000_000), "26M");
        assert_eq!(format_tokens(557_826_793), "557.8M");
        assert_eq!(format_tokens(2_500_000_000), "2.5B");
    }

    #[test]
    fn formats_pace_projection() {
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        let claude = payloads[1].pace.as_ref().unwrap();

        let primary = claude.primary.as_ref().unwrap();
        assert_eq!(primary.stage_text().as_deref(), Some("On pace"));
        assert_eq!(
            primary.projection_text().as_deref(),
            Some("Projected empty in 3h 49m")
        );

        // `willLastToReset` wins over any eta.
        let secondary = claude.secondary.as_ref().unwrap();
        assert_eq!(secondary.stage_text().as_deref(), Some("31% in reserve"));
        assert_eq!(
            secondary.projection_text().as_deref(),
            Some("Lasts until reset")
        );

        let codex = payloads[0].pace.as_ref().unwrap().secondary.as_ref().unwrap();
        assert_eq!(
            codex.projection_text().as_deref(),
            Some("Projected empty in 1d 4h")
        );
    }

    #[test]
    fn formats_snapshot_age_and_plan() {
        let payloads = parse_usage_json(REAL_WORLD).unwrap();
        let usage = payloads[0].usage.as_ref().unwrap();
        let updated: DateTime<Utc> = "2026-08-07T02:40:08Z".parse().unwrap();

        assert_eq!(usage.updated_text(updated).as_deref(), Some("Updated just now"));
        assert_eq!(
            usage.updated_text(updated + chrono::Duration::minutes(7)).as_deref(),
            Some("Updated 7m ago")
        );
        assert_eq!(
            usage.updated_text(updated + chrono::Duration::hours(5)).as_deref(),
            Some("Updated 5h ago")
        );
        assert_eq!(
            usage.updated_text(updated + chrono::Duration::days(3)).as_deref(),
            Some("Updated 3d ago")
        );
        // A snapshot dated in the future must not read as a negative age.
        assert_eq!(
            usage.updated_text(updated - chrono::Duration::hours(1)).as_deref(),
            Some("Updated just now")
        );

        assert_eq!(usage.plan_label().as_deref(), Some("Plus"));
        assert_eq!(
            payloads[2].usage.as_ref().unwrap().plan_label().as_deref(),
            Some("Antigravity Starter Quota")
        );
        // Claude's identity carries no loginMethod.
        assert!(payloads[1].usage.as_ref().unwrap().plan_label().is_none());
    }

    /// Claude's plan label is dropped whatever it says, and every other
    /// provider's is left alone.
    #[test]
    fn claude_reports_no_plan_label() {
        let scraped = r#"[{"provider": "claude", "source": "claude", "usage": {
            "loginMethod": "25",
            "identity": {"loginMethod": "25", "providerID": "claude"},
            "primary": {"usedPercent": 34, "windowMinutes": 300, "resetsAt": "2026-08-10T09:50:00Z"},
            "secondary": null, "tertiary": null,
            "updatedAt": "2026-08-10T08:31:55Z"}},
            {"provider": "codex", "usage": {"identity": {"loginMethod": "plus"}}}]"#;
        let payloads = parse_usage_json(scraped).expect("parses");
        assert!(payloads[0].plan_label().is_none());
        assert_eq!(payloads[1].plan_label().as_deref(), Some("Plus"));
        // Dropping the label leaves the windows beside it alone.
        let usage = payloads[0].usage.as_ref().unwrap();
        assert_eq!(usage.primary.as_ref().unwrap().window_label("Primary"), "Session");
    }

    #[test]
    fn pace_without_summary_has_no_lines() {
        let window = PaceWindow {
            stage: None,
            delta_percent: None,
            expected_used_percent: None,
            will_last_to_reset: None,
            eta_seconds: None,
            summary: None,
        };
        assert!(window.summary_lines().is_empty());
    }
}
