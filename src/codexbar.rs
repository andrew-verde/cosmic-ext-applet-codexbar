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
use std::process::Stdio;

use chrono::{DateTime, Utc};
use serde::Deserialize;

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

impl ProviderPayload {
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

    /// Reset text: the CLI's own description when present, otherwise a
    /// countdown computed from `resetsAt`.
    ///
    /// `resetDescription` is not consistently phrased across providers - Claude
    /// sends "Resets 3:50pm (Asia/Tokyo)" while Codex sends the bare
    /// "Aug 10 at 11:39 PM" - so the "Resets" prefix is only added when the
    /// description does not already carry it. Blindly prefixing produced
    /// "resets Resets 3:50pm (Asia/Tokyo)".
    pub fn reset_text(&self, now: DateTime<Utc>) -> Option<String> {
        if let Some(description) = &self.reset_description {
            let description = description.trim();
            if !description.is_empty() {
                if description.to_lowercase().starts_with("reset") {
                    return Some(description.to_string());
                }
                return Some(format!("Resets {description}"));
            }
        }
        let resets_at = self.resets_at?;
        let remaining = resets_at.signed_duration_since(now);
        if remaining.num_seconds() <= 0 {
            return Some("Resetting now".to_string());
        }
        let hours = remaining.num_hours();
        let minutes = remaining.num_minutes() % 60;
        if hours >= 24 {
            Some(format!("Resets in {}d {}h", hours / 24, hours % 24))
        } else if hours > 0 {
            Some(format!("Resets in {hours}h {minutes}m"))
        } else {
            Some(format!("Resets in {minutes}m"))
        }
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
    // cosmic-panel applets are launched by the systemd graphical session, which
    // does not source ~/.bashrc or ~/.profile, so managers that only extend PATH
    // there (Homebrew's `shellenv`, `~/.local/bin` added by some installers)
    // are invisible here even though `codexbar` works fine in a terminal.
    let mut candidates = vec!["codexbar".to_string(), dirs_local_bin()];
    candidates.extend(linuxbrew_candidates());

    let mut output = None;
    for candidate in &candidates {
        match run_cli(candidate).await {
            Ok(o) => {
                output = Some(o);
                break;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("could not run codexbar: {e}")),
        }
    }
    let Some(output) = output else {
        return Err(format!(
            "codexbar CLI not found on PATH, in ~/.local/bin, or in Homebrew's bin dir.\n\
             Install it from github.com/steipete/CodexBar."
        ));
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse_usage_json(&stdout) {
        Ok(payloads) => Ok(payloads),
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

async fn run_cli(program: &str) -> std::io::Result<std::process::Output> {
    tokio::process::Command::new(program)
        .args(["usage", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .await
}

fn dirs_local_bin() -> String {
    match std::env::var("HOME") {
        Ok(home) => format!("{home}/.local/bin/codexbar"),
        Err(_) => "codexbar".to_string(),
    }
}

/// Homebrew on Linux installs to `/home/linuxbrew/.linuxbrew` (the default,
/// shared prefix) or `~/.linuxbrew` (a per-user prefix used when the shared
/// one isn't writable). Its `bin/codexbar` is only a wrapper script that execs
/// the real binary under `Cellar/codexbar/<version>/`, so following the
/// `bin/codexbar` symlink is enough - no need to glob the Cellar directly.
fn linuxbrew_candidates() -> Vec<String> {
    let mut candidates = vec!["/home/linuxbrew/.linuxbrew/bin/codexbar".to_string()];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/.linuxbrew/bin/codexbar"));
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
        assert_eq!(
            secondary.reset_text(Utc::now()).as_deref(),
            Some("Resets Aug 10 at 11:39 PM")
        );

        let claude = &payloads[1];
        assert_eq!(claude.label(), "Claude");
        let claude_usage = claude.usage.as_ref().unwrap();
        assert_eq!(
            claude_usage.primary.as_ref().unwrap().window_label("P"),
            "Session"
        );
        // Claude's own description already starts with "Resets", so it is used
        // verbatim rather than double-labelled.
        assert_eq!(
            claude_usage
                .primary
                .as_ref()
                .unwrap()
                .reset_text(Utc::now())
                .as_deref(),
            Some("Resets 3:50pm (Asia/Tokyo)")
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
