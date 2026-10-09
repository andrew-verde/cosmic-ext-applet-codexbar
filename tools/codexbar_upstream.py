"""GitHub reads shared by icon sync and payload-source monitoring."""

import json
import os
from pathlib import Path
import re
import urllib.parse
import urllib.request

REPO = "steipete/CodexBar"
ROOT = Path(__file__).resolve().parent.parent


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "cosmic-ext-applet-codexbar"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def resolve_commit(ref):
    """Resolve a branch or tag once so reads use the same upstream tree."""
    reference = urllib.parse.quote(ref, safe="")
    commit = json.loads(fetch(f"https://api.github.com/repos/{REPO}/commits/{reference}"))["sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("GitHub returned an invalid commit SHA")
    return commit
