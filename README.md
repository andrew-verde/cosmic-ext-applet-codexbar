# CodexBar applet for COSMIC Desktop

A native [COSMIC](https://github.com/pop-os/cosmic-epoch) panel applet that shows
your OpenAI Codex / Claude Code usage limits, in the spirit of the macOS app
[CodexBar](https://github.com/steipete/CodexBar).

The applet adds a small icon to the COSMIC panel. Clicking (or hovering) it opens
a popup listing, for every provider CodexBar knows about:

- the provider label and account,
- session / weekly / monthly usage as a percentage plus a progress bar,
- when each limit window resets,
- remaining credits, when the provider reports them.

State is refreshed every 60 seconds, and again whenever the popup is opened.

## How it works

The applet does not talk to any provider itself. It shells out to the `codexbar`
CLI that ships with CodexBar:

```sh
codexbar usage --format json
```

and renders the resulting JSON. If the CLI is missing, fails, or reports an error
for a provider, the popup shows that error instead of going blank.

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
cd cosmic-ext-applet-codexbar
just build-release
sudo just install
```

`just install` places:

| file | destination |
| --- | --- |
| `cosmic-ext-applet-codexbar` | `/usr/bin/` |
| `dev.andrewgreen.codexbar.desktop` | `/usr/share/applications/` |
| `dev.andrewgreen.codexbar-symbolic.svg` | `/usr/share/icons/hicolor/scalable/apps/` |

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

## JSON schema notes

The parser in [`src/codexbar.rs`](src/codexbar.rs) targets the shape documented in
CodexBar's `docs/cli.md` and defined by `ProviderPayload` in
`Sources/CodexBarCLI/CLIPayloads.swift`: `codexbar usage --format json` emits a
JSON **array** of provider payloads, encoded by Swift's `JSONEncoder` with
lowerCamelCase keys and ISO 8601 dates.

Only the fields this applet displays are decoded — `provider`, `account`,
`version`, `source`, `usage.{primary,secondary,tertiary}.{usedPercent,
windowMinutes,resetsAt,resetDescription}`, `usage.updatedAt`, `credits.remaining`
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
