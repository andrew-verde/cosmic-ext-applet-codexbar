//! User configuration, read from
//! `$XDG_CONFIG_HOME/codexbar-cosmic-applet/config.toml` (in practice
//! `~/.config/codexbar-cosmic-applet/config.toml`).
//!
//! The file is optional: every field has a built-in default that reproduces the
//! applet's out-of-the-box appearance, so a missing or unreadable file is not an
//! error. A file that *is* present but malformed also falls back to the
//! defaults; the parse error is handed back to the caller so the popup can show
//! it instead of the applet silently ignoring the edit.
//!
//! [`load`] is called on every refresh tick rather than only at startup, so
//! edits apply within one poll interval without restarting the applet.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

/// Directory under `$XDG_CONFIG_HOME` holding `config.toml`.
const CONFIG_DIR: &str = "codexbar-cosmic-applet";

/// The file written on first run. Every field is present at its default value,
/// so this is also a faithful description of the built-in defaults.
pub const DEFAULT_CONFIG_TOML: &str = r#"# Configuration for the CodexBar COSMIC panel applet.
#
# Location: ~/.config/codexbar-cosmic-applet/config.toml
#           ($XDG_CONFIG_HOME/codexbar-cosmic-applet/config.toml)
#
# This file is re-read on every refresh, so edits apply within about 60 seconds
# with no need to restart the applet or the panel. Deleting the file restores
# the defaults below (a fresh copy is written out again on the next refresh).
#
# Every field is optional; anything left out keeps its default.

# Show the shortest rolling window (the "session" limit).
show_session = true

# Show the second window, normally the weekly limit.
show_weekly = true

# Show the third window, normally the monthly limit.
show_monthly = true

# Show the "resets in 2h 30m" text next to each window. When false the
# percentage and progress bar are still shown, only the reset text is dropped.
show_reset_countdown = true

# Show CodexBar's pace projection under each window, e.g.
# "On pace", "31% in reserve", "Projected empty in 3h 50m". Providers that do
# not report a projection are unaffected.
show_pace = true

# Show the cost / token block ("Today", "30d cost", "Latest tokens",
# "30d tokens"). Only Codex and Claude report cost data; other providers simply
# omit the block.
show_cost = true

# Show the remaining-credits line for providers that report credits.
show_credits = true

# Show the account (usually an email address) next to the provider name.
show_account = true

# Whether percentages and progress bars report quota consumed ("used", the
# default) or quota left ("remaining"). The popup always labels which mode is
# active. An unrecognised value falls back to "used".
usage_display = "used"

# Opacity of the popup background, from 0.0 (fully transparent) to 1.0
# (the default, unchanged COSMIC popup background). Values outside that range
# are clamped.
background_opacity = 1.0
"#;

/// User-editable settings. Defaults reproduce the applet's original behaviour.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Show `usage.primary`, the shortest rolling window.
    pub show_session: bool,
    /// Show `usage.secondary`, normally the weekly window.
    pub show_weekly: bool,
    /// Show `usage.tertiary`, normally the monthly window.
    pub show_monthly: bool,
    /// Show the reset countdown caption under each visible window.
    pub show_reset_countdown: bool,
    /// Show CodexBar's pace projection under each visible window.
    pub show_pace: bool,
    /// Show the cost / token block for providers that report cost data.
    pub show_cost: bool,
    /// Show the remaining-credits caption.
    pub show_credits: bool,
    /// Show the account caption in the provider header.
    pub show_account: bool,
    /// Report quota consumed or quota left.
    #[serde(deserialize_with = "deserialize_usage_display")]
    pub usage_display: UsageDisplay,
    /// Alpha multiplier for the popup background, clamped to `0.0..=1.0`.
    pub background_opacity: f32,
}

/// Whether a window's percentage and progress bar describe how much quota has
/// been consumed or how much is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageDisplay {
    #[default]
    Used,
    Remaining,
}

impl UsageDisplay {
    /// The percentage to display for a window that is `used_percent` consumed.
    pub fn percent(self, used_percent: f64) -> f64 {
        match self {
            UsageDisplay::Used => used_percent,
            UsageDisplay::Remaining => 100.0 - used_percent,
        }
    }

    /// The progress bar fraction matching [`UsageDisplay::percent`].
    pub fn fraction(self, used_fraction: f32) -> f32 {
        match self {
            UsageDisplay::Used => used_fraction,
            UsageDisplay::Remaining => 1.0 - used_fraction,
        }
    }

    /// Word for the active mode, shown in the popup so the numbers are never
    /// ambiguous.
    pub fn label(self) -> &'static str {
        match self {
            UsageDisplay::Used => "used",
            UsageDisplay::Remaining => "remaining",
        }
    }
}

/// Read `usage_display` leniently: anything other than `"remaining"` means
/// `"used"`, so a typo degrades to the default instead of rejecting the whole
/// file.
fn deserialize_usage_display<'de, D>(deserializer: D) -> Result<UsageDisplay, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(match raw.trim().to_lowercase().as_str() {
        "remaining" => UsageDisplay::Remaining,
        _ => UsageDisplay::Used,
    })
}

impl Default for Config {
    fn default() -> Self {
        Config {
            show_session: true,
            show_weekly: true,
            show_monthly: true,
            show_reset_countdown: true,
            show_pace: true,
            show_cost: true,
            show_credits: true,
            show_account: true,
            usage_display: UsageDisplay::Used,
            background_opacity: 1.0,
        }
    }
}

/// Parse the contents of `config.toml`.
///
/// Unknown keys are ignored and missing keys keep their default, so a config
/// written for an older or newer build of the applet still loads.
pub fn parse_config(contents: &str) -> Result<Config, String> {
    let mut config: Config =
        toml::from_str(contents).map_err(|e| format!("could not parse config.toml: {e}"))?;
    config.background_opacity = config.background_opacity.clamp(0.0, 1.0);
    Ok(config)
}

/// Path of the config file, or `None` when no config directory can be located.
pub fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(CONFIG_DIR).join("config.toml"))
}

/// Load the config, falling back to defaults.
///
/// Returns the config to use plus an optional message describing why the file
/// on disk was not honoured, so the caller can surface it in the popup. A
/// missing file is not a problem: [`DEFAULT_CONFIG_TOML`] is written out so the
/// user has something to edit, and failing to write it is reported but does not
/// change the config in use.
pub fn load() -> (Config, Option<String>) {
    match config_path() {
        Some(path) => load_from(&path),
        None => (Config::default(), None),
    }
}

/// [`load`] against an explicit path.
fn load_from(path: &Path) -> (Config, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(contents) => match parse_config(&contents) {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(format!("{error} (using defaults)"))),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (Config::default(), write_default(path).err())
        }
        Err(error) => (
            Config::default(),
            Some(format!("could not read {}: {error}", path.display())),
        ),
    }
}

/// Write [`DEFAULT_CONFIG_TOML`] to `path`, creating the directory if needed.
fn write_default(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, DEFAULT_CONFIG_TOML)
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything turned off and a non-default opacity.
    const CUSTOMISED: &str = r#"
        show_session = false
        show_weekly = false
        show_monthly = false
        show_reset_countdown = false
        show_pace = false
        show_cost = false
        show_credits = false
        show_account = false
        usage_display = "remaining"
        background_opacity = 0.75
    "#;

    /// A partial file: unset keys keep their defaults, unknown keys are ignored.
    const PARTIAL: &str = r#"
        show_credits = false
        future_option = "ignored"
    "#;

    #[test]
    fn defaults_preserve_original_behaviour() {
        let config = Config::default();
        assert!(config.show_session);
        assert!(config.show_weekly);
        assert!(config.show_monthly);
        assert!(config.show_reset_countdown);
        assert!(config.show_pace);
        assert!(config.show_cost);
        assert!(config.show_credits);
        assert!(config.show_account);
        assert_eq!(config.usage_display, UsageDisplay::Used);
        assert_eq!(config.background_opacity, 1.0);
    }

    #[test]
    fn parses_written_default_file() {
        assert_eq!(parse_config(DEFAULT_CONFIG_TOML).unwrap(), Config::default());
    }

    #[test]
    fn parses_customised_file() {
        let config = parse_config(CUSTOMISED).unwrap();
        assert!(!config.show_session);
        assert!(!config.show_weekly);
        assert!(!config.show_monthly);
        assert!(!config.show_reset_countdown);
        assert!(!config.show_pace);
        assert!(!config.show_cost);
        assert!(!config.show_credits);
        assert!(!config.show_account);
        assert_eq!(config.usage_display, UsageDisplay::Remaining);
        assert_eq!(config.background_opacity, 0.75);
    }

    #[test]
    fn parses_both_usage_display_modes() {
        assert_eq!(
            parse_config(r#"usage_display = "used""#)
                .unwrap()
                .usage_display,
            UsageDisplay::Used
        );
        assert_eq!(
            parse_config(r#"usage_display = "remaining""#)
                .unwrap()
                .usage_display,
            UsageDisplay::Remaining
        );
    }

    #[test]
    fn unrecognised_usage_display_falls_back_to_used() {
        let config = parse_config(r#"usage_display = "reamining""#).unwrap();
        assert_eq!(config.usage_display, UsageDisplay::Used);
        // The rest of the file still applies.
        assert!(config.show_pace);
    }

    #[test]
    fn usage_display_inverts_only_in_remaining_mode() {
        assert_eq!(UsageDisplay::Used.percent(28.0), 28.0);
        assert_eq!(UsageDisplay::Remaining.percent(28.0), 72.0);
        assert!((UsageDisplay::Used.fraction(0.28) - 0.28).abs() < 1e-6);
        assert!((UsageDisplay::Remaining.fraction(0.28) - 0.72).abs() < 1e-6);
        assert_eq!(UsageDisplay::Used.label(), "used");
        assert_eq!(UsageDisplay::Remaining.label(), "remaining");
    }

    #[test]
    fn missing_keys_keep_defaults() {
        let config = parse_config(PARTIAL).unwrap();
        assert!(!config.show_credits);
        assert!(config.show_session);
        assert_eq!(config.background_opacity, 1.0);
    }

    #[test]
    fn empty_file_is_all_defaults() {
        assert_eq!(parse_config("").unwrap(), Config::default());
    }

    #[test]
    fn clamps_out_of_range_opacity() {
        assert_eq!(
            parse_config("background_opacity = 4.0")
                .unwrap()
                .background_opacity,
            1.0
        );
        assert_eq!(
            parse_config("background_opacity = -1.0")
                .unwrap()
                .background_opacity,
            0.0
        );
    }

    /// A private scratch directory, so these tests never touch the real
    /// `~/.config/codexbar-cosmic-applet`.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codexbar-applet-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("config.toml")
    }

    #[test]
    fn missing_file_yields_defaults_and_writes_one() {
        let path = scratch("missing");
        let (config, error) = load_from(&path);
        assert_eq!(config, Config::default());
        assert_eq!(error, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG_TOML);

        // The file it wrote loads back as the defaults it was written from.
        assert_eq!(load_from(&path), (Config::default(), None));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn malformed_file_yields_defaults_and_surfaces_the_error() {
        let path = scratch("malformed");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "show_session = yes").unwrap();

        let (config, error) = load_from(&path);
        assert_eq!(config, Config::default());
        let error = error.expect("parse error should be surfaced");
        assert!(error.starts_with("could not parse config.toml"));
        assert!(error.ends_with("(using defaults)"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn valid_file_is_honoured() {
        let path = scratch("valid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, CUSTOMISED).unwrap();

        let (config, error) = load_from(&path);
        assert_eq!(error, None);
        assert!(!config.show_weekly);
        assert_eq!(config.background_opacity, 0.75);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reports_malformed_file() {
        let error = parse_config("show_session = yes").unwrap_err();
        assert!(error.starts_with("could not parse config.toml"));
        assert!(parse_config("show_credits = 3").is_err());
        assert!(parse_config("[[[").is_err());
    }
}
