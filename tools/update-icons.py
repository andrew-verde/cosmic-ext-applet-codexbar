#!/usr/bin/env python3
"""Sync provider icons from one CodexBar commit and validate icon-only changes.

Run with `just update-icons`. Weekly automation validates and tests changes
before committing them. Only the Python standard library is required.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import re
import subprocess
import sys
import urllib.error
import xml.etree.ElementTree as ET

from codexbar_upstream import REPO, ROOT, fetch, resolve_commit

RESOURCE_PATH = "Sources/CodexBar/Resources"

#: Upstream names every icon file this way; the rest of the name is the slug,
#: which is also the provider id the CLI reports.
PREFIX = "ProviderIcon-"
SUFFIX = ".svg"

ICON_DIR = ROOT / "data" / "icons" / "providers"
ICONS_RS = ROOT / "src" / "icons.rs"

#: The script owns everything between these two lines in `src/icons.rs`.
BEGIN = "// BEGIN GENERATED ICON TABLE"
END = "// END GENERATED ICON TABLE"


def upstream_icons(commit):
    """Map provider IDs to canonical download URLs at a pinned commit."""
    url = f"https://api.github.com/repos/{REPO}/contents/{RESOURCE_PATH}?ref={commit}"
    listing = json.loads(fetch(url))
    icons = {
        entry["name"][len(PREFIX) : -len(SUFFIX)]:
            f"https://raw.githubusercontent.com/{REPO}/{commit}/{RESOURCE_PATH}/{entry['name']}"
        for entry in listing
        if entry["name"].startswith(PREFIX) and entry["name"].endswith(SUFFIX)
    }
    # A listing with no icons at all means the path moved or the request was
    # throttled into an error object. Deleting every vendored icon on the back
    # of that would be worse than doing nothing.
    if not icons:
        raise ValueError(f"no provider icons at {url}; refusing to wipe the set")
    # Slugs end up in a Rust string literal and a file path, and the table is
    # sorted bytewise for `provider_icon`'s binary search. Anything outside this
    # set is worth a human look rather than a generated file that may not build.
    odd = sorted(slug for slug in icons if not re.fullmatch(r"[a-z0-9_]+", slug))
    if odd:
        raise ValueError(f"unexpected characters in provider slugs: {', '.join(odd)}")
    return icons


def validate_svg(content, slug):
    """Require SVG XML with no scripts, event handlers or external references."""
    root = ET.fromstring(content)
    if root.tag.rsplit("}", 1)[-1] != "svg":
        raise ValueError(f"{slug} is not an SVG")
    for element in root.iter():
        if element.tag.rsplit("}", 1)[-1] in ("script", "foreignObject"):
            raise ValueError(f"{slug} contains active content")
        for key, value in element.attrib.items():
            name = key.rsplit("}", 1)[-1].lower()
            if name.startswith("on") or (name == "href" and not value.startswith("#")):
                raise ValueError(f"{slug} contains an event handler or external reference")


def sync(icons):
    """Download and validate the whole set before replacing or deleting files."""
    if not icons or any(not re.fullmatch(r"[a-z0-9_]+", slug) for slug in icons):
        raise ValueError("invalid or empty icon set")

    def download(slug):
        content = fetch(icons[slug])
        validate_svg(content, slug)
        return slug, content

    with ThreadPoolExecutor(max_workers=8) as pool:
        contents = dict(pool.map(download, sorted(icons)))
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    local = {path.stem for path in ICON_DIR.glob(f"*{SUFFIX}")}

    added, updated = [], []
    for slug in sorted(icons):
        path = ICON_DIR / f"{slug}{SUFFIX}"
        content = contents[slug]
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


def validate_staged():
    """Permit only SVG changes and the exact generated table in the staged diff."""
    names = subprocess.check_output(
        ["git", "diff", "--cached", "--name-only", "-z"], cwd=ROOT
    ).decode().split("\0")
    for name in filter(None, names):
        if name != "src/icons.rs" and not re.fullmatch(r"data/icons/providers/[a-z0-9_]+\.svg", name):
            raise ValueError(f"unexpected staged file: {name}")
    icons = sorted(ICON_DIR.glob("*.svg"))
    if not icons:
        raise ValueError("refusing an empty local icon set")
    for path in icons:
        if path.is_symlink() or not re.fullmatch(r"[a-z0-9_]+", path.stem):
            raise ValueError(f"invalid icon path: {path.name}")
        validate_svg(path.read_bytes(), path.stem)
    previous = subprocess.check_output(["git", "show", "HEAD:src/icons.rs"], cwd=ROOT).decode()
    pattern = re.compile(f"{re.escape(BEGIN)}.*?{re.escape(END)}", re.DOTALL)
    if len(pattern.findall(previous)) != 1:
        raise ValueError("expected one generated icon table")
    expected = pattern.sub(lambda _: render_table([path.stem for path in icons]), previous)
    if ICONS_RS.read_text() != expected:
        raise ValueError("src/icons.rs differs outside the generated table or does not match the icon set")
    # Validation must cover exactly what will be committed, including file modes.
    if subprocess.check_output(["git", "diff", "--", "data/icons/providers", "src/icons.rs"], cwd=ROOT):
        raise ValueError("unstaged icon changes remain")
    raw = subprocess.check_output(["git", "diff", "--cached", "--raw", "--no-abbrev"], cwd=ROOT).decode()
    for entry in raw.splitlines():
        modes = entry.split()[:2]
        if any(mode not in (":000000", ":100644", "000000", "100644") for mode in modes):
            raise ValueError("icon updates must contain regular files only")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ref", default="main", help="upstream branch, tag or commit")
    parser.add_argument("--validate-staged", action="store_true", help="validate a staged icon-only update without network access")
    args = parser.parse_args()
    if args.validate_staged:
        validate_staged()
        print("staged icon update is valid")
        return
    commit = resolve_commit(args.ref)
    icons = upstream_icons(commit)
    added, updated, removed = sync(icons)
    rewrote = regenerate(sorted(icons))

    print(f"upstream {commit} has {len(icons)} provider icons")
    for label, slugs in (("added", added), ("updated", updated), ("removed", removed)):
        if slugs:
            print(f"{label} {len(slugs)}: {', '.join(slugs)}")
    if not (added or updated or removed):
        print("no icon changes")
    print(f"{ICONS_RS.relative_to(ROOT)}: {'regenerated' if rewrote else 'already up to date'}")


if __name__ == "__main__":
    try:
        main()
    except (urllib.error.URLError, ValueError, ET.ParseError, subprocess.CalledProcessError) as error:
        sys.exit(f"icon update failed: {error}")
