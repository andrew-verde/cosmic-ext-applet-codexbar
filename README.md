# codexbar-cosmic-applet

A native [COSMIC](https://github.com/pop-os/cosmic-epoch) panel applet that shows
your OpenAI Codex / Claude Code usage limits, in the spirit of the macOS app
[CodexBar](https://github.com/steipete/CodexBar).

The applet adds a small icon to the COSMIC panel. Clicking it opens a popup with
a tab per provider — each showing that provider's logo over its name — plus an
**Overview** tab that condenses every provider to one line. The tab strip
scrolls horizontally, so any number of providers fits. A provider's own tab
shows:

- the provider label, account, snapshot age and plan,
- session / weekly / monthly usage as a percentage plus a progress bar,
- when each limit window resets,
- CodexBar's pace projection ("On pace", "Projected empty in 3h 50m"),
- today's and the last 30 days' cost and token counts,
- remaining credits, when the provider reports them.

The popup body scrolls, so extra providers or windows never push content out of
view. State is refreshed every 60 seconds, and again whenever the popup is
opened.

## How it works

The applet does not talk to any provider itself. It shells out to the `codexbar`
CLI that ships with CodexBar:

```sh
codexbar usage --format json
codexbar cost --format json --days 30
```

and renders the resulting JSON. If the CLI is missing, fails, or reports an error
for a provider, the popup shows that error instead of going blank. Only Codex and
Claude appear in the `cost` output; providers missing from it simply have no cost
block. A failing `cost` call never blanks the usage display.

## Prerequisites

- The COSMIC desktop (this is a `cosmic-panel` applet; it does not run under
  GNOME, KDE, or Cinnamon).
- A Rust toolchain and [`just`](https://github.com/casey/just).
- The `codexbar` CLI, installed separately from
  [steipete/CodexBar](https://github.com/steipete/CodexBar) (Homebrew, the AUR, or
  a release tarball). The applet looks for `codexbar` on `PATH` first and then
  falls back to `~/.local/bin/codexbar`.
- At least one provider enabled in CodexBar, e.g.:

  ```sh
  codexbar config enable --provider codex
  codexbar config enable --provider claude
  ```

On Fedora, `just` is available via `sudo dnf install just`.

## Build and install

```sh
git clone <this repository>
cd codexbar-cosmic-applet
just build-release
sudo just install
```

`just install` places:

| file | destination |
| --- | --- |
| `codexbar-cosmic-applet` | `/usr/bin/` |
| `io.github.andrew-verde.CodexBarCosmicApplet.desktop` | `/usr/share/applications/` |
| `io.github.andrew-verde.CodexBarCosmicApplet-symbolic.svg` | `/usr/share/icons/hicolor/scalable/apps/` |

To install somewhere else, override `prefix` or `rootdir`, e.g.
`just prefix=$HOME/.local install` (COSMIC also reads applets from
`~/.local/share/applications`).

Remove it again with `sudo just uninstall`. Run the unit tests with `just test`.

## Adding it to the panel

1. Open **Settings → Desktop → Panel** (or **Dock**).
2. Choose **Configure panel applets**.
3. Find **CodexBar** in the list and add it to whichever section you prefer.

You may need to log out and back in (or restart `cosmic-panel`) before a
newly installed applet appears in that list.

## Configuration

The applet reads an optional TOML file from

```
~/.config/codexbar-cosmic-applet/config.toml
```

(strictly `$XDG_CONFIG_HOME/codexbar-cosmic-applet/config.toml`). It is
written out with every field at its default the first time the applet runs, and
re-read on every refresh — edits apply within about 60 seconds, with no need to
restart the applet or the panel. If the file is missing or malformed the applet
falls back to the defaults; a parse error is shown as a caption at the bottom of
the popup rather than being swallowed.

| field | type | default | effect |
| --- | --- | --- | --- |
| `show_session` | bool | `true` | Show the shortest rolling window (`usage.primary`). |
| `show_weekly` | bool | `true` | Show the second window (`usage.secondary`), normally weekly. |
| `show_monthly` | bool | `true` | Show the third window (`usage.tertiary`), normally monthly. |
| `show_reset_countdown` | bool | `true` | Show the "Resets in 2h 30m" line under each visible window. When `false` the percentage and progress bar remain. |
| `show_pace` | bool | `true` | Show CodexBar's pace projection under each visible window, e.g. "On pace", "31% in reserve", "Projected empty in 3h 50m". Providers that report no projection are unaffected. |
| `show_cost` | bool | `true` | Show the cost / token block ("Today", "30d cost", "Latest tokens", "30d tokens"). Only Codex and Claude report cost data; other providers omit the block. |
| `show_credits` | bool | `true` | Show the remaining-credits line for providers that report credits. |
| `show_account` | bool | `true` | Show the account (usually an email address) beside the provider name. |
| `usage_display` | string | `"used"` | `"used"` reports quota consumed, `"remaining"` reports quota left (percentages and bars are inverted). Each line names the mode, e.g. "20% used". An unrecognised value falls back to `"used"`. |
| `background_opacity` | float | *unset* | Alpha of the popup background, from `0.0` (fully transparent) to `1.0` (solid). Leave it out to follow the COSMIC theme, which is what makes the popup look like every other panel popup — translucent when "frosted applets" is on so the compositor blurs behind it, opaque when it is off. Setting a value overrides the theme outright; use `1.0` if a translucent popup is hard to read over a busy wallpaper. Out-of-range values are clamped. |

Unknown keys are ignored and omitted keys keep their default, so the defaults
above are also exactly the behaviour with no config file at all.

## JSON schema notes

The parser in [`src/codexbar.rs`](src/codexbar.rs) targets the shape documented in
CodexBar's `docs/cli.md` and defined by `ProviderPayload` in
`Sources/CodexBarCLI/CLIPayloads.swift`: `codexbar usage --format json` emits a
JSON **array** of provider payloads, encoded by Swift's `JSONEncoder` with
lowerCamelCase keys and ISO 8601 dates.

Only the fields this applet displays are decoded — `provider`, `account`,
`version`, `source`, `usage.{primary,secondary,tertiary}.{usedPercent,
windowMinutes,resetsAt,resetDescription}`, `usage.updatedAt`,
`pace.{primary,secondary,tertiary}`, `credits.remaining`
and `error.message`. Everything is optional and unknown keys are ignored, so a
CodexBar release that adds or renames fields degrades gracefully rather than
breaking the applet.

Two derived pieces of presentation are *not* in the JSON and are computed here:

- **Provider labels.** The payload carries only the provider id, so `codex` and
  `claude` are mapped to "Codex" and "Claude" and anything else is capitalised.
- **Window names.** "Session" / "Weekly" / "Monthly" are derived from
  `windowMinutes` (`<= 300`, `10080`, `43200`); other values are rendered
  generically, and a missing `windowMinutes` falls back to
  Primary/Secondary/Tertiary.

## License

MIT — see [LICENSE](LICENSE).

The provider icons under `data/icons/providers/` are vendored from
[CodexBar](https://github.com/steipete/CodexBar) (MIT, Copyright (c) 2026 Peter
Steinberger) and embedded into the binary at compile time. See
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for the full license text.
