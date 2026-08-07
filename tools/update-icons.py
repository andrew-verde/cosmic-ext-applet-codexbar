#!/usr/bin/env python3
"""Re-vendor CodexBar's provider icons and regenerate the table in `src/icons.rs`.

The applet's data path is generic - it renders whatever providers the `codexbar`
CLI reports - but the brand icons are a snapshot of
`Sources/CodexBar/Resources/ProviderIcon-<slug>.svg` in steipete/CodexBar, so a
provider added upstream has no logo here until someone re-vendors. This script
is that someone: it syncs `data/icons/providers/` with upstream and rewrites the
`PROVIDER_ICONS` array so it stays sorted, which is what
`provider_icon`'s binary search depends on.

Run it with `just update-icons`; CI runs it weekly and opens a pull request.
Only the standard library is used, so there is nothing to install first.

Set `GITHUB_TOKEN` to raise the API rate limit. It is optional: the listing is a
single request and the downloads come from raw.githubusercontent.com.
"""

import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO = "steipete/CodexBar"
LISTING_URL = f"https://api.github.com/repos/{REPO}/contents/Sources/CodexBar/Resources"

#: Upstream names every icon file this way; the rest of the name is the slug,
#: which is also the provider id the CLI reports.
PREFIX = "ProviderIcon-"
SUFFIX = ".svg"

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / "data" / "icons" / "providers"
ICONS_RS = ROOT / "src" / "icons.rs"

#: The script owns everything between these two lines in `src/icons.rs`.
BEGIN = "// BEGIN GENERATED ICON TABLE"
END = "// END GENERATED ICON TABLE"


def fetch(url):
    """GET `url`, authenticating when a token is in the environment."""
    request = urllib.request.Request(url, headers={"User-Agent": "codexbar-cosmic-applet"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def upstream_icons():
    """Map slug -> download URL for every provider icon upstream currently has."""
    import json

    listing = json.loads(fetch(LISTING_URL))
    icons = {
        entry["name"][len(PREFIX) : -len(SUFFIX)]: entry["download_url"]
        for entry in listing
        if entry["name"].startswith(PREFIX) and entry["name"].endswith(SUFFIX)
    }
    # A listing with no icons at all means the path moved or the request was
    # throttled into an error object. Deleting every vendored icon on the back
    # of that would be worse than doing nothing.
    if not icons:
        sys.exit(f"no {PREFIX}*{SUFFIX} files at {LISTING_URL} - refusing to wipe the set")
    # Slugs end up in a Rust string literal and a file path, and the table is
    # sorted bytewise for `provider_icon`'s binary search. Anything outside this
    # set is worth a human look rather than a generated file that may not build.
    odd = sorted(slug for slug in icons if not re.fullmatch(r"[a-z0-9_]+", slug))
    if odd:
        sys.exit(f"unexpected characters in provider slugs: {', '.join(odd)}")
    return icons


def sync(icons):
    """Bring `ICON_DIR` in line with upstream. Returns (added, updated, removed)."""
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    local = {path.stem for path in ICON_DIR.glob(f"*{SUFFIX}")}

    added, updated = [], []
    for slug in sorted(icons):
        path = ICON_DIR / f"{slug}{SUFFIX}"
        content = fetch(icons[slug])
        # Compare content, not just presence: upstream redraws icons too.
        if not path.exists():
            added.append(slug)
        elif path.read_bytes() == content:
            continue
        else:
            updated.append(slug)
        path.write_bytes(content)

    removed = sorted(local - set(icons))
    for slug in removed:
        (ICON_DIR / f"{slug}{SUFFIX}").unlink()

    return added, updated, removed


def render_table(slugs):
    """The generated block: a doc comment plus the sorted `PROVIDER_ICONS` array."""
    lines = [
        BEGIN,
        "/// Every vendored icon, keyed by provider id. Sorted, so lookup can bisect.",
        "const PROVIDER_ICONS: &[(&str, &[u8])] = &[",
    ]
    lines += [
        f'    ("{slug}", include_bytes!("../data/icons/providers/{slug}{SUFFIX}")),'
        for slug in slugs
    ]
    lines += ["];", END]
    return "\n".join(lines)


def regenerate(slugs):
    """Replace the generated block in `src/icons.rs`, leaving the rest alone."""
    source = ICONS_RS.read_text()
    pattern = re.compile(f"{re.escape(BEGIN)}.*?{re.escape(END)}", re.DOTALL)
    if not pattern.search(source):
        sys.exit(f"{ICONS_RS} has no {BEGIN} / {END} block to regenerate")
    updated = pattern.sub(lambda _: render_table(slugs), source)
    if updated != source:
        ICONS_RS.write_text(updated)
        return True
    return False


def main():
    icons = upstream_icons()
    added, updated, removed = sync(icons)
    rewrote = regenerate(sorted(icons))

    print(f"upstream has {len(icons)} provider icons")
    for label, slugs in (("added", added), ("updated", updated), ("removed", removed)):
        if slugs:
            print(f"{label} {len(slugs)}: {', '.join(slugs)}")
    if not (added or updated or removed):
        print("no icon changes")
    print(f"{ICONS_RS.relative_to(ROOT)}: {'regenerated' if rewrote else 'already up to date'}")


if __name__ == "__main__":
    try:
        main()
    except urllib.error.URLError as error:
        sys.exit(f"could not reach GitHub: {error}")
