#!/usr/bin/env python3
"""
doc_age_audit.py — File age audit for doc/research retirement.

Scans a directory tree, reports the 'last touched' age of each file,
and flags candidates for removal.

Uses `git log` to determine when a file was last modified (more meaningful
than filesystem mtime). Falls back to filesystem mtime if git is unavailable.

Usage:
    python scripts/doc_age_audit.py                     # default: scan docs/
    python scripts/doc_age_audit.py docs/research       # scan specific dir
    python scripts/doc_age_audit.py --threshold 60      # flag files older than 60 days
    python scripts/doc_age_audit.py --remove-candidates # only show flagged files
    python scripts/doc_age_audit.py --exclude "*.toml*" --exclude "index.md"
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def find_git_root(start: Path) -> Path | None:
    """Walk up from `start` to find the git repo root."""
    current = start.resolve()
    while current != current.parent:
        if (current / ".git").exists():
            return current
        current = current.parent
    return None


def git_last_touched(filepath: Path, git_root: Path) -> datetime | None:
    """Return the datetime of the last git commit that modified `filepath`."""
    try:
        result = subprocess.run(
            ["git", "log", "-1", "--format=%aI", "--", str(filepath)],
            cwd=git_root,
            capture_output=True,
            text=True,
            timeout=10,
        )
        date_str = result.stdout.strip()
        if date_str:
            return datetime.fromisoformat(date_str)
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        pass
    return None


def fs_last_touched(filepath: Path) -> datetime:
    """Return the filesystem mtime of `filepath` as a datetime."""
    mtime = filepath.stat().st_mtime
    return datetime.fromtimestamp(mtime, tz=timezone.utc)


def collect_files(
    root: Path,
    exclude_patterns: list[str],
) -> list[Path]:
    """Walk `root` and return all files, excluding those matching any pattern."""
    files: list[Path] = []
    for dirpath, _dirnames, filenames in os.walk(root):
        for fname in filenames:
            if any(fnmatch.fnmatch(fname, pat) for pat in exclude_patterns):
                continue
            files.append(Path(dirpath) / fname)
    return files


def format_age(days: int) -> str:
    """Format an age in days as a human-friendly string, right-aligned."""
    if days >= 365:
        years = days / 365.25
        return f"{years:.1f}y"
    return f"{days}d"


def main() -> int:
    parser = argparse.ArgumentParser(
        description="File age audit for doc/research retirement.",
    )
    parser.add_argument(
        "directory",
        nargs="?",
        default="docs",
        help="Directory to scan (default: docs/)",
    )
    parser.add_argument(
        "--threshold",
        type=int,
        default=90,
        help="Flag files older than this many days (default: 90)",
    )
    parser.add_argument(
        "--remove-candidates",
        action="store_true",
        help="Only show files older than the threshold",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="Glob pattern(s) for filenames to exclude (repeatable)",
    )
    args = parser.parse_args()

    scan_dir = Path(args.directory)
    if not scan_dir.is_dir():
        print(f"Error: '{scan_dir}' is not a directory.", file=sys.stderr)
        return 1

    git_root = find_git_root(scan_dir)
    use_git = git_root is not None
    if not use_git:
        print(
            "Warning: not inside a git repo, falling back to filesystem mtime.\n",
            file=sys.stderr,
        )

    files = collect_files(scan_dir, args.exclude)
    if not files:
        print("No files found.")
        return 0

    now = datetime.now(tz=timezone.utc)

    # Collect (age_days, last_date, path) tuples
    entries: list[tuple[int, datetime, Path]] = []
    for fpath in files:
        if use_git:
            assert git_root is not None
            dt = git_last_touched(fpath, git_root)
            if dt is None:
                # File not tracked by git — fall back to fs
                dt = fs_last_touched(fpath)
        else:
            dt = fs_last_touched(fpath)

        # Normalize to UTC for comparison
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        age_days = (now - dt).days
        entries.append((age_days, dt, fpath))

    # Sort oldest first
    entries.sort(key=lambda e: e[0], reverse=True)

    # Filter if --remove-candidates
    if args.remove_candidates:
        entries = [(a, d, p) for a, d, p in entries if a >= args.threshold]
        if not entries:
            print(f"No files older than {args.threshold} days.")
            return 0

    # Print header
    flag_col = "FLAG"
    header = f"  {'AGE':>6}  {'LAST TOUCHED':12}  {flag_col:4}  PATH"
    print(header)
    print(f"  {'─' * 6}  {'─' * 12}  {'─' * 4}  {'─' * 40}")

    flagged_count = 0
    for age_days, dt, fpath in entries:
        age_str = format_age(age_days)
        date_str = dt.strftime("%Y-%m-%d")
        flag = " ██ " if age_days >= args.threshold else "    "
        if age_days >= args.threshold:
            flagged_count += 1
        print(f"  {age_str:>6}  {date_str:12}  {flag}  {fpath}")

    # Summary
    print()
    total = len(entries) if not args.remove_candidates else flagged_count
    print(
        f"  {len(entries)} files scanned, {flagged_count} flagged (≥{args.threshold} days)"
    )

    if flagged_count and not args.remove_candidates:
        print(f"\n  Tip: re-run with --remove-candidates to see only flagged files.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
