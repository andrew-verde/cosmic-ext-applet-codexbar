#!/usr/bin/env python3
"""Compare the latest CodexBar release's payload sources with the reviewed baseline."""

import argparse
import json
from pathlib import Path
import re
import sys
import urllib.error

from codexbar_upstream import REPO, ROOT, fetch, resolve_commit

BASELINE = ROOT / "tools" / "upstream-schema.json"


def source_hashes(commit, paths):
    tree = json.loads(fetch(f"https://api.github.com/repos/{REPO}/git/trees/{commit}?recursive=1"))
    if tree.get("truncated"):
        raise ValueError("upstream tree was truncated")
    blobs = {item["path"]: item["sha"] for item in tree["tree"] if item["type"] == "blob"}
    return {path: blobs.get(path) for path in paths}


def changed_files(baseline, current):
    return [path for path, digest in baseline["files"].items() if current.get(path) != digest]


def report(baseline, release, commit, changed):
    lines = [
        "<!-- codexbar-schema-monitor -->",
        f"CodexBar {release} changed payload source files since the reviewed {baseline['ref']} baseline.",
        "",
        "Review the diff against the applet parser. Source changes can be internal changes rather than a JSON format change.",
        "",
    ]
    for path in changed:
        lines.append(f"- [`{path}`](https://github.com/{REPO}/blob/{commit}/{path})")
    lines += [
        "",
        f"[Upstream diff](https://github.com/{REPO}/compare/{baseline['commit']}...{commit})",
        "",
        "After reviewing the parser and fixtures, accept the baseline with:",
        "",
        "```sh",
        f"python3 tools/check-upstream-schema.py --accept {release}",
        "```",
        "",
        "Commit the updated baseline before closing this issue. Icon updates continue independently.",
    ]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept", help="record a reviewed upstream tag or commit")
    parser.add_argument("--report", type=Path, help="write the drift report without creating an issue")
    parser.add_argument("--github-output", type=Path, help="append the changed flag for Actions")
    args = parser.parse_args()
    baseline = json.loads(BASELINE.read_text())
    if args.accept:
        ref = args.accept
    else:
        ref = json.loads(fetch(f"https://api.github.com/repos/{REPO}/releases/latest"))["tag_name"]
    if not re.fullmatch(r"[A-Za-z0-9._-]+", ref):
        raise ValueError("invalid upstream reference")
    commit = resolve_commit(ref)
    current = source_hashes(commit, baseline["files"])
    if args.accept:
        if any(digest is None for digest in current.values()):
            raise ValueError("a monitored file is missing; review the monitored paths first")
        BASELINE.write_text(json.dumps({"ref": ref, "commit": commit, "files": current}, indent=2) + "\n")
        print(f"recorded reviewed baseline {ref}")
        return
    changed = changed_files(baseline, current)
    if args.report:
        args.report.write_text(report(baseline, ref, commit, changed) if changed else "")
    if args.github_output:
        with args.github_output.open("a") as output:
            output.write(f"changed={'true' if changed else 'false'}\n")
    print(f"{ref}: {len(changed)} monitored source files changed")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, urllib.error.URLError) as error:
        sys.exit(f"schema check failed: {error}")
