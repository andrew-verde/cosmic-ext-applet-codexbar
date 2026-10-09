# Development

Build a release binary with `just build-release`. Run the Rust tests with `just test` and the maintenance script tests with:

```sh
python3 -m unittest discover -s tools/tests -p 'test_*.py'
```

## Provider icons

The applet creates tabs for provider IDs returned by CodexBar. Its logos are SVG files copied into `data/icons/providers/` and embedded in the binary. Providers without an icon get a text label.

Run `just update-icons` to sync the current upstream icons and regenerate the lookup table in [`src/icons.rs`](../src/icons.rs). Each sync resolves one upstream commit, downloads and validates every icon, then replaces the local files. A failed download leaves the local set unchanged. To use a specific upstream tag or commit:

```sh
python3 tools/update-icons.py --ref v0.73.0
```

The [weekly workflow](../.github/workflows/update-icons.yml) runs the script tests, syncs icons, and stages only the SVG files and generated lookup table. It checks that no other code changed, then runs the Rust tests before committing directly to `main`. Icon updates no longer need a pull request or manual merge. A concurrent update to `main` makes the push fail without overwriting it. The workflow can also run manually from GitHub Actions.

To validate a staged icon-only update locally:

```sh
git add data/icons/providers src/icons.rs
python3 tools/update-icons.py --validate-staged
```

## Upstream format changes

The same workflow compares payload source files in the latest stable CodexBar release with the reviewed hashes in [`tools/upstream-schema.json`](../tools/upstream-schema.json). It opens or updates one issue when a watched file changes or disappears. A source change may leave the JSON format unchanged, so this is a review prompt rather than a compatibility test.

Run the check locally with:

```sh
python3 tools/check-upstream-schema.py --report /tmp/codexbar-schema-report.md
```

After reviewing an upstream release and updating the parser or fixtures as needed, record its baseline:

```sh
python3 tools/check-upstream-schema.py --accept v0.73.0
```

Commit the baseline before closing the issue. Review the watched paths first if an upstream file moved. Schema review does not block icon updates.

## Payload parser

The parser in [`src/codexbar.rs`](../src/codexbar.rs) targets the JSON formats emitted by `codexbar usage --format json` and `codexbar cost`. See [CLI integration and data format](data-format.md) for decoded fields and display fallbacks.
