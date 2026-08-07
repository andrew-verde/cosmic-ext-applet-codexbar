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
    pub fn reset_text(&self, now: DateTime<Utc>) -> Option<String> {
        if let Some(description) = &self.reset_description {
            if !description.is_empty() {
                return Some(format!("resets {description}"));
            }
        }
        let resets_at = self.resets_at?;
        let remaining = resets_at.signed_duration_since(now);
        if remaining.num_seconds() <= 0 {
            return Some("resetting now".to_string());
        }
        let hours = remaining.num_hours();
        let minutes = remaining.num_minutes() % 60;
        if hours >= 24 {
            Some(format!("resets in {}d {}h", hours / 24, hours % 24))
        } else if hours > 0 {
            Some(format!("resets in {hours}h {minutes}m"))
        } else {
            Some(format!("resets in {minutes}m"))
        }
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
    let output = match run_cli("codexbar").await {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let fallback = dirs_local_bin();
            match run_cli(&fallback).await {
                Ok(output) => output,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    return Err(
                        "codexbar CLI not found on PATH or in ~/.local/bin.\n\
                         Install it from github.com/steipete/CodexBar."
                            .to_string(),
                    );
                }
                Err(e) => return Err(format!("could not run codexbar: {e}")),
            }
        }
        Err(e) => return Err(format!("could not run codexbar: {e}")),
        Ok(output) => output,
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
            Some("resets today at 3:00 PM")
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
        assert_eq!(primary.reset_text(now).as_deref(), Some("resets in 2h 0m"));

        let secondary = payloads[0]
            .usage
            .as_ref()
            .unwrap()
            .secondary
            .as_ref()
            .unwrap();
        assert_eq!(
            secondary.reset_text(now).as_deref(),
            Some("resets in 23h 45m")
        );
    }
}
