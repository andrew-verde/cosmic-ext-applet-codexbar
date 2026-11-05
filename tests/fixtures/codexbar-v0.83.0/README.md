# CodexBar v0.83.0 contract review

These are hand-authored examples with invented accounts and values, not captured
CLI output. They exercise the applet's usage and cost parsers against the reviewed
v0.83.0 types at commit `348e2fa8d36ae8e452739ca07a79e83ab9d25e98`.

Sources and review against the previous v0.73.0 baseline:

- [CLIPayloads.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCLI/CLIPayloads.swift) adds optional `ageSeconds`. Dashboard account transport metadata is excluded from encoding. Existing usage, labels, status, credits, error, and pace fields retain their types.
- [UsageFetcher.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCore/UsageFetcher.swift) adds window `title`, `quotaLimit`, and `quotaUnit`, plus snapshot metadata such as plugin branding, menu items, throughput, subscription date flags, and plan provenance. The applet ignores these fields. Changes to RPC fetching and reconciliation do not change the fields it decodes.
- [ProviderDetailSection.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCore/ProviderDetailSection.swift) adds optional row `severity` and progress `expectedUsed`. The fixture checks that their presence preserves detail text. The applet still ignores row styling and progress metadata.
- [CLICostCommand.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCLI/CLICostCommand.swift) retains the default cost array and the six fields the applet reads. New identified-usage flags select a separate output mode that the applet does not request. Other changes concern text rendering.
- [CostUsageModels.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCore/CostUsageModels.swift) moves a session breakdown type and adds internal completeness and model-cost provenance metadata. The displayed CLI cost fields retain their types.
- [CreditsModels.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCore/CreditsModels.swift) is unchanged.

[CLIRenderer.swift](https://github.com/steipete/CodexBar/blob/348e2fa8d36ae8e452739ca07a79e83ab9d25e98/Sources/CodexBarCLI/CLIRenderer.swift) still rounds pace `etaSeconds` to whole seconds before encoding, matching the applet's integer field.

No parser changes were needed for these source changes. These fixtures verify
decoding and existing display helpers; they do not verify live provider fetching
or every optional upstream field. Older and partial payloads remain covered by
the unit tests in `src/codexbar.rs`.
