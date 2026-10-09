# Development

Build a release binary with `just build-release`. Run the unit tests with `just test`.

## Provider icons

The applet creates tabs for provider IDs returned by CodexBar. Its logos are SVG files copied from CodexBar into `data/icons/providers/` and embedded in the binary. Providers without a copied icon still get a text label.

Run `just update-icons` to copy the current upstream icons and regenerate the lookup table in [`src/icons.rs`](../src/icons.rs). The repository's scheduled icon update automation is being revised. Documentation of its behavior is pending that update; check the workflow before relying on it.

## Payload parser

The parser in [`src/codexbar.rs`](../src/codexbar.rs) targets the JSON formats emitted by `codexbar usage --format json` and `codexbar cost`. See [CLI integration and data format](data-format.md) for the fields and fallbacks the applet uses.
