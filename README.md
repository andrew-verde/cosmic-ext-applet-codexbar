# cosmic-ext-applet-codexbar

A COSMIC panel applet that displays usage limits reported by the [CodexBar CLI](https://github.com/steipete/CodexBar). Each provider gets a tab. The Overview tab summarizes providers, and accounts for the same provider stack together. The applet refreshes every 60 seconds and when its popup opens.

This is a third-party project. It is not official COSMIC software and is not endorsed by System76. See the [trademark notice](docs/install.md#trademarks) and [license information](docs/install.md#license).

![Applet popup showing Claude usage windows, pace, and cost and token counts](docs/screenshot.png)

![Overview tab showing provider usage summaries](docs/screenshot-overview.png)

Install the COSMIC desktop, Rust, `just`, and the CodexBar CLI. Follow the [build and installation guide](docs/install.md). See [account and display settings](docs/configuration.md) to configure the applet.

## Documentation

- [Installation and panel setup](docs/install.md)
- [Configuration and multiple accounts](docs/configuration.md)
- [CLI integration and data format](docs/data-format.md)
- [Development](docs/development.md)
- [Third-party licenses](THIRD_PARTY_LICENSES.md)
