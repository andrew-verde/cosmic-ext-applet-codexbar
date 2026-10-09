# CLI integration and data format

The applet does not connect to AI providers. It runs the `codexbar` command line tool and displays its JSON output:

```sh
codexbar usage --format json
codexbar cost --format json --days 30
```

CodexBar must be installed on the host. If the command is missing, fails, or reports a provider error, the popup shows an error. A failed cost query does not clear usage data. Providers without cost data have no cost block.

## Usage payload

`codexbar usage --format json` returns a JSON array of provider payloads. The applet decodes the fields it displays and ignores unknown fields. Optional fields can be absent. The parser expects lower camel case keys and ISO 8601 dates, as produced by Swift's `JSONEncoder`.

The displayed fields are:

- Provider, account, version, and source.
- `usage.primary`, `usage.secondary`, and `usage.tertiary`, including each window's `usedPercent`, `windowMinutes`, `resetsAt`, and `resetDescription`.
- `usage.updatedAt` and `rateWindowLabels.primary`, `.secondary`, and `.tertiary`.
- `usage.identity.loginMethod` and `usage.identity.accountEmail`.
- `usage.codexResetCredits.credits`, including each credit's `status` and `expires_at`.
- `pace.primary`, `.secondary`, and `.tertiary`; `credits.remaining`; and `error.message`.

The account caption uses `usage.identity.accountEmail`, falling back to the top-level `account` field. CodexBar fills `account` when it queries multiple accounts.

For cost output, the applet reads `provider`, `currencyCode`, `sessionCostUSD`, `sessionTokens`, `last30DaysCostUSD`, and `last30DaysTokens`.

## Display labels and fallbacks

CodexBar provides provider IDs, so the applet maps `codex` and `claude` to "Codex" and "Claude" and capitalizes other IDs. The current payload also provides `rateWindowLabels`, which the applet uses as window names. For missing or blank labels, it uses `windowMinutes` to choose Session, Weekly, Monthly, or a duration label. If the duration is missing, it uses the slot name. Older CLI versions without `rateWindowLabels` use these fallbacks.

When `resetsAt` is present, the applet calculates a countdown and prefers it to `resetDescription`. If the timestamp is absent, it uses the description and removes any parenthesized timezone. Token counts use abbreviations, for example `19.5M`.

The reset-credit structure is not documented in CodexBar's CLI guide. The applet follows the Swift source and live payload shape, where the keys in this nested object use snake case. It counts credits with `status == "available"` that have not expired, matching the macOS app's behavior.

See [CodexBar's CLI documentation](https://github.com/steipete/CodexBar/blob/v0.73.0/docs/cli.md) and [`ProviderPayload`](https://github.com/steipete/CodexBar/blob/v0.73.0/Sources/CodexBarCLI/CLIPayloads.swift) for the upstream definitions. The applet parser is in [`src/codexbar.rs`](../src/codexbar.rs).
