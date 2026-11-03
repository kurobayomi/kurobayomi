#!/usr/bin/env python3
"""The app's GitHub owner, repository name and app ID, kept in one place: identity.toml.

    scripts/set-identity.py --owner kurobayomi             # move the repository
    scripts/set-identity.py --app-id io.github.kurobayomi.Kurobayomi
    scripts/set-identity.py --check                        # CI: every file agrees with identity.toml

A change rewrites every tracked text file that names the old values (`owner/repo`, `|owner|repo|` in the AppImage update
info, the app ID dotted, as a D-Bus path and regex-escaped), renames the files whose names hold the app ID with
`git mv`, then writes identity.toml. Files that record history name the old values on purpose and are left alone.
"""

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IDENTITY = ROOT / "identity.toml"
# history, which names earlier identities on purpose, and this script, whose usage names examples
SKIP = ("docs/audit/", "docs/ORG-MIGRATION.md", "identity.toml", "scripts/set-identity.py")


def tracked() -> list[str]:
    out = subprocess.run(["git", "ls-files"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return [p for p in out.splitlines() if not p.startswith(SKIP)]


def text_of(path: str) -> str | None:
    try:
        return (ROOT / path).read_text(encoding="utf-8")
    except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError):
        return None


def app_id_forms(app_id: str) -> list[str]:
    """The app ID as files write it: dotted, as a D-Bus or resource path, and escaped for a regex."""
    return [app_id, "/" + app_id.replace(".", "/"), app_id.replace(".", "\\.")]


def repo_pattern(owner: str, repo: str) -> re.Pattern:
    # `owner/repo` as a whole name: not part of a longer path segment or of `repo-something`
    return re.compile(rf"(?<![\w.-]){re.escape(owner)}/{re.escape(repo)}(?![\w-])")


def rewrite(text: str, old: dict, new: dict) -> str:
    for before, after in zip(app_id_forms(old["app_id"]), app_id_forms(new["app_id"])):
        text = text.replace(before, after)
    text = repo_pattern(old["owner"], old["repo"]).sub(f"{new['owner']}/{new['repo']}", text)
    return text.replace(f"|{old['owner']}|{old['repo']}|", f"|{new['owner']}|{new['repo']}|")


def apply(old: dict, new: dict) -> None:
    for path in tracked():
        text = text_of(path)
        if text is not None and (changed := rewrite(text, old, new)) != text:
            (ROOT / path).write_text(changed, encoding="utf-8")
            print(f"rewrote {path}")
    for path in tracked():
        if old["app_id"] in path and old["app_id"] != new["app_id"]:
            target = path.replace(old["app_id"], new["app_id"])
            (ROOT / target).parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "mv", path, target], cwd=ROOT, check=True)
            print(f"renamed {path} -> {target}")
    IDENTITY.write_text(rewrite_identity(IDENTITY.read_text(encoding="utf-8"), new), encoding="utf-8")


def rewrite_identity(text: str, new: dict) -> str:
    for key, value in new.items():
        text = re.sub(rf'^{key} = ".*"$', f'{key} = "{value}"', text, flags=re.M)
    return text


def problems(identity: dict) -> list[str]:
    """Every place a tracked file names an app ID or a GitHub owner of this repository other than identity.toml's."""
    owner, repo, app_id = identity["owner"], identity["repo"], identity["app_id"]
    prefix, name = app_id.rsplit(".", 1)
    # an app ID is `<reverse domain>.<name>`: the same name after any other reverse domain is a stale one
    ids = re.compile(rf"(?<![\w.])([\w-]+(?:\.[\w-]+)+)\.{re.escape(name)}(?![\w-])")
    # the owner in a GitHub link, a shields.io badge, an API path or the update setting of a workflow
    owners = re.compile(rf"(?:github\.com/|shields\.io/github/[\w/-]*/|repos/|KB_UPDATE_REPOSITORY: )([\w-]+)/{re.escape(repo)}(?![\w-])")
    found = []
    for path in tracked():
        found += [f"{path}: the file name holds app ID {m.group(0)}" for m in ids.finditer(path) if m.group(1) != prefix]
        text = text_of(path)
        for line_no, line in enumerate((text or "").splitlines(), 1):
            found += [f"{path}:{line_no}: app ID {m.group(0)}" for m in ids.finditer(line) if m.group(1) != prefix]
            found += [f"{path}:{line_no}: GitHub owner {m.group(1)}" for m in owners.finditer(line) if m.group(1) != owner]
            if f"|{repo}|" in line and f"|{owner}|{repo}|" not in line:
                found.append(f"{path}:{line_no}: AppImage update info names another owner")
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--owner")
    parser.add_argument("--repo")
    parser.add_argument("--app-id")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    old = tomllib.loads(IDENTITY.read_text(encoding="utf-8"))
    if args.check:
        found = problems(old)
        for line in found:
            print(line, file=sys.stderr)
        print("FAILED: run scripts/set-identity.py, or fix the lines above" if found else "OK: every file agrees with identity.toml")
        return 1 if found else 0
    new = {"owner": args.owner or old["owner"], "repo": args.repo or old["repo"], "app_id": args.app_id or old["app_id"]}
    if new == old:
        print("identity.toml already says that")
        return 0
    apply(old, new)
    return 0


if __name__ == "__main__":
    sys.exit(main())
