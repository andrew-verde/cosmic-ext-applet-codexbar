# Installation

## Requirements

- A COSMIC desktop session. This is a `cosmic-panel` applet and does not run in GNOME, KDE, or Cinnamon.
- Rust and [`just`](https://github.com/casey/just). Install them through your distribution or install `just` with `cargo install just`.
- The `codexbar` CLI, installed separately from [steipete/CodexBar](https://github.com/steipete/CodexBar). CodexBar offers Homebrew, AUR, and release tarball installs.
- At least one provider enabled in CodexBar. For example:

  ```sh
  codexbar config enable --provider codex
  codexbar config enable --provider claude
  ```

The applet searches for `codexbar` on `PATH`, then in `~/.local/bin`, `/home/linuxbrew/.linuxbrew/bin`, and `~/.linuxbrew/bin`. The graphical session may not load the shell profile that sets your `PATH`.

## Build and install from source

```sh
git clone https://github.com/andrew-verde/cosmic-ext-applet-codexbar.git
cd cosmic-ext-applet-codexbar
just build-release
sudo just install
```

The install recipe places the executable in `/usr/bin`, the desktop file in `/usr/share/applications`, the icons in `/usr/share/icons/hicolor/scalable/apps`, and the applet metadata in `/usr/share/metainfo`.

To install under another prefix, set `prefix` or `rootdir`. For example, `just prefix=$HOME/.local install` uses the user's local directory. COSMIC also reads applets from `~/.local/share/applications`. Remove a system install with `sudo just uninstall`.

## Flatpak build

The Flatpak manifest is at `flatpak/io.github.andrew_verde.cosmic-ext-applet-codexbar.json`. Install the builder and SDKs:

```sh
flatpak install flathub org.flatpak.Builder
flatpak install flathub com.system76.Cosmic.BaseApp//stable \
    org.freedesktop.Sdk//25.08 org.freedesktop.Sdk.Extension.rust-stable//25.08
```

Build and install it with:

```sh
cd flatpak
flatpak run --filesystem=host --share=network \
    --env=FLATPAK_USER_DIR="$HOME/.local/share/flatpak" \
    --command=flatpak-builder org.flatpak.Builder \
    --user --force-clean \
    build io.github.andrew_verde.cosmic-ext-applet-codexbar.json

flatpak build-export .flatpak-builder/cache build master
flatpak install --user --reinstall "$PWD/.flatpak-builder/cache" \
    io.github.andrew_verde.cosmic-ext-applet-codexbar
```

`FLATPAK_USER_DIR` lets the builder find the installed COSMIC base app. The manifest uses `flatpak/cargo-sources.json` to build Rust dependencies offline. Regenerate that file after changing `Cargo.lock`:

```sh
flatpak run --filesystem=host --command=flatpak-cargo-generator \
    org.flatpak.Builder Cargo.lock -o flatpak/cargo-sources.json
```

CodexBar must be installed on the host because it reads provider credentials from host directories such as `~/.codex` and `~/.claude`. The sandbox runs the CLI through `flatpak-spawn --host`. It reads applet settings from the host's `~/.config/cosmic-ext-applet-codexbar/config.toml`.

The host `flatpak build-export` command above corrects a launcher path that the builder Flatpak can export as `/app/bin/flatpak`. The installed launcher must use the host's `/usr/bin/flatpak`.

Keep the launcher under Flatpak's export directory. Do not copy it to `~/.local/share/applications` as an override. COSMIC identifies Flatpak applets by the launcher path and passes their panel socket only when it recognizes that path. A native launcher override can make the icon open as a floating desktop window. See [COSMIC's Flatpak detection](https://github.com/pop-os/cosmic-panel/blob/master/cosmic-panel-bin/src/space/panel_space.rs#L203-L207).

## Add the applet to the panel

1. Open **Settings → Desktop → Panel** or **Dock**.
2. Choose **Configure panel applets**.
3. Find **CodexBar** and add it to a panel section.

A new applet may not appear in the list until you log out and back in. Restarting `cosmic-panel` may not be enough. If upgrading from an applet ID with `io.github.andrew-verde.*`, remove the old applet in panel settings and add the applet again. The hyphen in that ID position is not valid in a Flatpak ID.

## License

The applet is MIT licensed. See [LICENSE](../LICENSE). The provider icons under `data/icons/providers/` are unmodified copies from [CodexBar](https://github.com/steipete/CodexBar), which is also MIT licensed. One function in `src/window.rs` derives from libcosmic code and is covered by MPL-2.0. See [third-party license details](../THIRD_PARTY_LICENSES.md).

## Trademarks

COSMIC™ is a trademark of System76, Inc. This third-party applet integrates with the COSMIC desktop. It is not affiliated with or endorsed by System76. See the [COSMIC trademark policy](https://github.com/pop-os/cosmic-epoch/blob/master/TRADEMARK.md).
