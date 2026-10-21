#!/usr/bin/env python3
"""Documentation handoff report.

Fails on broken links. Reports, without failing, the signals of documentation
accretion: word growth per changed guide, prose paragraphs over 200 words and
hedge-pattern density. Run before handing off documentation changes; findings
are for judgment, not automatic verdicts.
"""

import os
import re
import subprocess
import sys

EXCLUDED_DIRS = {".git", ".worktrees", "node_modules", ".cache", "target", "samples"}
HEDGE_PATTERNS = [
    r"\bdoes not establish\b",
    r"\bdoes not (?:imply|mean|guarantee|constitute)\b",
    r"\bis not a (?:gate|requirement|substitute)\b",
    r"\bare not (?:gates|requirements)\b",
    r"\b(?:should not|not to) be confused\b",
    r"\bit is worth noting\b",
    r"\bkeep in mind\b",
    r"\bremains (?:unverified|deferred|future work)\b",
]
LONG_PARAGRAPH_WORDS = 200
STRUCTURAL_LINE = re.compile(r"^(?:#{1,6}\s|[-*+]\s|\d+[.)]\s\S|\||>|```)")
SKIPPED_FILES = {"WORK_PLAN.md"}


def md_files(root):
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in EXCLUDED_DIRS]
        files.extend(
            os.path.normpath(os.path.join(dirpath, f))
            for f in filenames
            if f.endswith(".md") and os.path.normpath(os.path.join(dirpath, f)) not in SKIPPED_FILES
        )
    return sorted(files)


def read(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")


def fail(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def slug(heading):
    text = re.sub(r"[^\w\s-]", "", heading.strip().strip("#").strip())
    return re.sub(r"[\s_]+", "-", text.strip()).lower()


def check_links(files):
    anchors = {
        p: {slug(m.group(2)) for m in re.finditer(r"^(#{1,6})\s+(.*)", read(p), re.M)}
        for p in files
    }
    broken = []
    for p in files:
        for m in re.finditer(r"\[[^\]]*\]\(([^)]+)\)", read(p)):
            target_url = m.group(1)
            if target_url.startswith(("http", "mailto:", "<")):
                continue
            path_part = target_url.split("#")[0]
            if not path_part:
                continue
            target = os.path.normpath(os.path.join(os.path.dirname(p), path_part))
            if not os.path.exists(target):
                broken.append(f"{p}: missing file: {target_url}")
            elif "#" in target_url and target in anchors:
                if target_url.split("#", 1)[1] not in anchors[target]:
                    broken.append(f"{p}: missing anchor: {target_url}")
    return broken


def tracked_word_count(path):
    result = subprocess.run(
        ["git", "show", f"HEAD:{path}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return None
    return len(result.stdout.split())


def report_word_growth(files):
    changed = subprocess.run(
        ["git", "diff", "--name-only", "HEAD", "--", "*.md"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    tracked = set(os.path.normpath(p) for p in changed)
    print("\nWord growth vs HEAD (guides grow by accretion; justify any +):")
    found = False
    for path in files:
        if path in SKIPPED_FILES:
            continue
        new = len(read(path).split())
        old = tracked_word_count(path)
        if old is None:
            is_guide = os.path.basename(path) == "README.md" or path.startswith(
                ("docs/", ".pi/skills/")
            )
            if is_guide:
                print(f"  +{new:5d}  {path}  (new)")
                found = True
            continue
        if path in tracked and new != old:
            print(f"  {new - old:+5d}  {path}  ({old} -> {new})")
            found = True
    if not found:
        print("  no tracked guide changed")


def report_long_paragraphs(files):
    print("\nParagraphs over %d words:" % LONG_PARAGRAPH_WORDS)
    found = False
    for path in files:
        if path in SKIPPED_FILES:
            continue
        start = None
        words = 0
        for lineno, line in enumerate(read(path).splitlines(), 1):
            if line.strip() and not STRUCTURAL_LINE.match(line.strip()):
                if start is None:
                    start = lineno
                words += len(line.split())
                continue
            if start is not None and words > LONG_PARAGRAPH_WORDS:
                print(f"  {path}:{start}  {words} words")
                found = True
            start, words = None, 0
    if not found:
        print("  none")


def report_hedges(files):
    print("\nHedge-pattern matches (boundary sentences can be genuine; judge each):")
    found = False
    for path in files:
        for lineno, line in enumerate(read(path).splitlines(), 1):
            for pattern in HEDGE_PATTERNS:
                if re.search(pattern, line, re.I):
                    print(f"  {path}:{lineno}  {pattern}: {line.strip()[:100]}")
                    found = True
    if not found:
        print("  none")


def main():
    if not os.path.isdir(".git"):
        fail("run from the repository root")
    files = md_files(".")
    broken = check_links(files)
    print(f"{len(files)} markdown files checked")
    report_word_growth(files)
    report_long_paragraphs(files)
    report_hedges(files)
    print(f"\nbroken links: {len(broken)}")
    for entry in broken:
        print(f"  {entry}")
    if broken:
        sys.exit(1)


if __name__ == "__main__":
    main()
