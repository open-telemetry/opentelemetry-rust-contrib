#!/usr/bin/env python3
"""Ping component owners when a new issue is opened.

Parses the "What component are you working with?" dropdown from the issue
body, looks up owners in ``.github/component_owners.yml``, applies
``comp:<crate>`` labels, and posts a single comment pinging the owners.
Crates without explicit owners fall back to the default repo approvers team.

Environment variables (set by the workflow):
    GITHUB_TOKEN       - token used by ``gh`` for API access
    GITHUB_REPOSITORY  - owner/repo, e.g. open-telemetry/opentelemetry-rust-contrib
    ISSUE_NUMBER       - the new issue number
    ISSUE_BODY         - raw markdown body of the issue
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

import yaml

OWNERS_FILE = Path(".github/component_owners.yml")
COMPONENT_HEADING = "What component are you working with?"
COMP_LABEL_COLOR = "0e8a16"
NO_OWNER_LABEL = "triage:no-owner"
NO_OWNER_LABEL_COLOR = "d93f0b"
DEFAULT_OWNER = "open-telemetry/rust-approvers"


def load_owners() -> dict[str, list[str]]:
    raw = yaml.safe_load(OWNERS_FILE.read_text()) or {}
    components = raw.get("components") or {}
    return {
        path.rstrip("/"): list(owners or [])
        for path, owners in components.items()
    }


# GitHub renders multi-select dropdown answers as a single comma-separated
# line today, but accept newline- or bullet-separated forms too so this
# script doesn't silently break if that rendering ever changes.
_COMPONENT_SPLIT = re.compile(r"[,\n]+")
_BULLET_PREFIX = re.compile(r"^[-*]\s+")


def parse_components(body: str) -> list[str]:
    """Return the dropdown values selected for the component question.

    Returns an empty list when the section is missing, blank, or only N/A.
    """
    pattern = rf"###\s+{re.escape(COMPONENT_HEADING)}\s*\n+(.+?)(?=\n###|\Z)"
    m = re.search(pattern, body, re.DOTALL)
    if not m:
        return []
    value = m.group(1).strip()
    if value in {"", "_No response_", "N/A"}:
        return []
    out: list[str] = []
    for raw in _COMPONENT_SPLIT.split(value):
        token = _BULLET_PREFIX.sub("", raw).strip()
        if token and token != "N/A" and token not in out:
            out.append(token)
    return out


def gh(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], check=check, text=True)


def ensure_label(repo: str, name: str, color: str) -> None:
    # `gh label create` returns non-zero if the label already exists; that
    # is the desired behaviour — we never want to clobber an existing color
    # or description, just guarantee the label is present.
    # Let stderr through: "already exists" is harmless noise,
    # but other failures should be visible in workflow logs.
    subprocess.run(
        ["gh", "label", "create", name, "-R", repo, "--color", color],
        check=False,
        text=True,
        stdout=subprocess.DEVNULL,
    )


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    issue = os.environ["ISSUE_NUMBER"]
    body = os.environ.get("ISSUE_BODY", "") or ""

    selected = parse_components(body)
    if not selected:
        print("No component selected; nothing to do.")
        return 0

    owners = load_owners()
    known = [c for c in selected if c in owners]
    unknown = [c for c in selected if c not in owners]
    if unknown:
        print(f"Selected components not in component_owners.yml: {unknown}")
    if not known:
        print("No known components selected; nothing to do.")
        return 0

    comp_labels = [f"comp:{c}" for c in known]
    for label in comp_labels:
        ensure_label(repo, label, COMP_LABEL_COLOR)

    labels = list(comp_labels)
    no_owner = [c for c in known if not owners[c]]
    if no_owner:
        ensure_label(repo, NO_OWNER_LABEL, NO_OWNER_LABEL_COLOR)
        labels.append(NO_OWNER_LABEL)
    gh("issue", "edit", issue, "-R", repo, "--add-label", ",".join(labels))

    lines = []
    for c in known:
        component_owners = owners[c] or [DEFAULT_OWNER]
        mentions = " ".join(f"@{u}" for u in component_owners)
        lines.append(f"- `{c}`: {mentions}")

    comment = "Pinging component owners:\n" + "\n".join(lines)
    gh("issue", "comment", issue, "-R", repo, "--body", comment)
    return 0


if __name__ == "__main__":
    sys.exit(main())
