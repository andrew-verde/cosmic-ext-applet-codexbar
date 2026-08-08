#!/usr/bin/env python3
"""Derive the cosmic-flatpak submission manifest from the local build manifest.

`flatpak/<app-id>.json` builds from a `dir` source so it can be built against
the working tree. The copy that lives in pop-os/cosmic-flatpak has to build from
a pinned commit in this repository instead, because their builder only has the
manifest, not a checkout.

Everything else - permissions, install commands, runtime versions - is copied
across untouched, so the local manifest stays the single place any of it is
edited and a release cannot quietly ship different `finish-args` than the ones
that were tested here.
"""

import argparse
import json
import sys
from pathlib import Path

APP_ID = "io.github.andrew_verde.codexbar-cosmic-applet"
REPO_URL = "https://github.com/andrew-verde/codexbar-cosmic-applet.git"


def swap_source(manifest: dict, tag: str, commit: str) -> None:
    """Replace the `dir` source with a git source pinned to `commit`.

    Raises if there is not exactly one, rather than emitting a manifest that
    would build the wrong tree: a submission that silently builds from `..` on
    someone else's builder is worse than a failed release.
    """
    replaced = 0
    for module in manifest.get("modules", []):
        if not isinstance(module, dict):
            continue
        for index, source in enumerate(module.get("sources", [])):
            if isinstance(source, dict) and source.get("type") == "dir":
                module["sources"][index] = {
                    "type": "git",
                    "url": REPO_URL,
                    "tag": tag,
                    "commit": commit,
                }
                replaced += 1
    if replaced != 1:
        sys.exit(
            f"expected exactly one 'dir' source to pin, found {replaced}. "
            "The local manifest's sources have changed shape; update this script."
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="release tag, e.g. v0.1.0")
    parser.add_argument("--commit", required=True, help="full commit SHA the tag points at")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path(f"flatpak/{APP_ID}.json"),
        help="the local build manifest to derive from",
    )
    parser.add_argument("-o", "--output", type=Path, help="write here instead of stdout")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text())
    swap_source(manifest, args.tag, args.commit)

    # Trailing newline and two-space indent to match the manifests already in
    # cosmic-flatpak, so the submission diff is about the content and not style.
    rendered = json.dumps(manifest, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered)
    else:
        sys.stdout.write(rendered)


if __name__ == "__main__":
    main()
