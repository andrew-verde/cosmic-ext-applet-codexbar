# Configuration and multiple accounts

The applet reads an optional TOML file from:

```text
~/.config/cosmic-ext-applet-codexbar/config.toml
```

This is `$XDG_CONFIG_HOME/cosmic-ext-applet-codexbar/config.toml` outside Flatpak. Inside Flatpak, the applet reads the host user's `~/.config` path. On first run, it writes a commented configuration file with the default values. It reloads the file on each refresh, about every 60 seconds. A malformed file makes the applet use defaults and show the parse error in the popup.

Every setting is optional. Missing settings keep their defaults, and unknown keys are ignored.

| Setting | Type | Default | Effect |
| --- | --- | --- | --- |
| `show_session` | Boolean | `true` | Show the shortest rolling usage window (`usage.primary`). |
| `show_weekly` | Boolean | `true` | Show the second usage window (`usage.secondary`), normally weekly. |
| `show_monthly` | Boolean | `true` | Show the third usage window (`usage.tertiary`), normally monthly. |
| `show_reset_countdown` | Boolean | `true` | Show the time remaining until each visible window resets. Percentages and bars remain visible when this is `false`. |
| `show_pace` | Boolean | `true` | Show CodexBar's pace projection for each visible window, when the provider reports one. |
| `show_cost` | Boolean | `true` | Show cost and token counts when CodexBar reports them. The CLI currently reports these for Codex and Claude. |
| `show_reset_credits` | Boolean | `true` | Show redeemable Codex limit-reset credits when any are available. |
| `show_credits` | Boolean | `true` | Show remaining credits when a provider reports them. |
| `show_account` | Boolean | `true` | Show the account name, usually an email address, beside the provider name. |
| `account_labels` | Table | `{}` | Assign short names to account sections. Keys are account emails or labels reported by CodexBar. |
| `usage_display` | String | `"used"` | Use `"used"` for quota consumed or `"remaining"` for quota left. The popup labels the selected mode. Other values use `"used"`. |
| `background_opacity` | Number | Unset | Set popup alpha from `0.0` (transparent) to `1.0` (opaque). When unset, the popup follows the COSMIC theme. Out-of-range values are clamped. |

For example:

```toml
usage_display = "remaining"
show_cost = false
account_labels = { "personal@example.com" = "Personal", "team@example.com" = "Virufy" }
background_opacity = 1.0
```

## Multiple accounts

Each provider has one tab. If CodexBar reports multiple accounts for a provider, each account becomes a row under the provider name. A collapsed row shows a small bar for the account's most-used window and a line summarizing its windows. Click a row to expand it to the account's full usage, pace, credits, and errors. A provider tab opens its first account. The Overview tab starts with every row collapsed. Each view remembers its own expanded rows while the applet runs. A failure for one account does not hide the others.

The applet uses CodexBar's account configuration. It includes Codex accounts from `codexProfileHomePaths`. For other enabled providers, it includes configured `tokenAccounts` when at least two are present. See [CodexBar account configuration](https://github.com/steipete/CodexBar/blob/v0.73.0/docs/configuration.md).

`account_labels` keys match the email or account label in the CLI data. If a key does not match, the applet uses a non-email CLI label, then the email. Setting `show_account = false` replaces emails with numbered names such as "Account 1". Custom names stay visible.

CodexBar does not split local log costs by account. The cost block appears once per provider and describes activity on the machine, not an account bill. Additional Codex homes may not be included in that scan.
