#!/usr/bin/env python3
"""Move a repository's legacy .waap data into its central state worktree."""

from __future__ import annotations

import argparse
import filecmp
import os
from pathlib import Path
import shutil
import subprocess
import sys

STATE_DIRS = ("agents", "tickets")


def run(*args: str, cwd: Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def repository_root(cwd: Path) -> Path:
    top = Path(run("git", "rev-parse", "--show-toplevel", cwd=cwd).stdout.strip()).resolve()
    common = Path(
        run("git", "rev-parse", "--path-format=absolute", "--git-common-dir", cwd=cwd)
        .stdout.strip()
    ).resolve()
    primary = common.parent
    if common != primary / ".git" or top != primary:
        raise RuntimeError("run this script from the primary repository checkout")
    return primary


def state_directory(repository: Path) -> Path:
    home = Path.home()
    return home / ".local/state/waap/data" / repository.relative_to(repository.anchor)


def source_files(legacy: Path) -> list[Path]:
    unsupported = sorted(
        entry.name
        for entry in legacy.iterdir()
        if entry.name not in {*STATE_DIRS, ".gitkeep"}
    )
    if unsupported:
        raise RuntimeError(
            "unsupported legacy top-level entries: " + ", ".join(unsupported)
        )

    files: list[Path] = []
    for directory in STATE_DIRS:
        root = legacy / directory
        if not root.exists():
            continue
        if root.is_symlink() or not root.is_dir():
            raise RuntimeError(f"{root} must be a directory, not a symlink")
        for current, directories, names in os.walk(root):
            current_path = Path(current)
            for name in directories:
                path = current_path / name
                if path.is_symlink():
                    raise RuntimeError(f"cannot migrate symlink {path}")
            for name in names:
                path = current_path / name
                if path.is_symlink() or not path.is_file():
                    raise RuntimeError(f"cannot migrate non-file {path}")
                if name != ".gitkeep":
                    files.append(path)
    return sorted(files)


def migration_plan(legacy: Path, state: Path) -> tuple[list[tuple[Path, Path]], list[Path]]:
    copies: list[tuple[Path, Path]] = []
    conflicts: list[Path] = []
    for source in source_files(legacy):
        relative = source.relative_to(legacy)
        destination = state / relative
        if not destination.exists():
            copies.append((source, destination))
        elif destination.is_file() and filecmp.cmp(source, destination, shallow=False):
            continue
        else:
            conflicts.append(relative)
    return copies, conflicts


def initialize_state(repository: Path, state: Path, waap_bin: str) -> None:
    if not state.exists():
        result = run(waap_bin, "init", cwd=repository, check=False)
        if result.returncode:
            raise RuntimeError(f"waap init failed: {result.stderr.strip()}")
    if not state.is_dir():
        raise RuntimeError(f"state path {state} is not a directory")
    branch = run("git", "branch", "--show-current", cwd=state).stdout.strip()
    common = Path(
        run("git", "rev-parse", "--path-format=absolute", "--git-common-dir", cwd=state)
        .stdout.strip()
    ).resolve()
    if branch != "waap" or common != (repository / ".git").resolve():
        raise RuntimeError(f"{state} is not this repository's waap state worktree")
    if any(not (state / directory).is_dir() for directory in STATE_DIRS):
        raise RuntimeError(f"state directory {state} must contain agents and tickets")


def commit_copies(state: Path, destinations: list[Path]) -> None:
    if not destinations:
        return
    relative = [str(path.relative_to(state)) for path in destinations]
    run("git", "add", "--", *relative, cwd=state)
    changed = run("git", "diff", "--cached", "--quiet", "--", *relative, cwd=state, check=False)
    if changed.returncode == 1:
        run("git", "commit", "-m", "Migrate legacy .waap state", "--", *relative, cwd=state)
    elif changed.returncode != 0:
        raise RuntimeError(changed.stderr.strip() or "failed to inspect staged state changes")


def remove_legacy(repository: Path, legacy: Path) -> None:
    tracked = run("git", "ls-files", "--", ".waap", cwd=repository).stdout.splitlines()
    shutil.rmtree(legacy)
    if not tracked:
        return
    run("git", "add", "-A", "--", ".waap", cwd=repository)
    changed = run(
        "git", "diff", "--cached", "--quiet", "--", ".waap", cwd=repository, check=False
    )
    if changed.returncode == 1:
        run(
            "git",
            "commit",
            "-m",
            "Remove legacy waap state",
            "--",
            ".waap",
            cwd=repository,
        )
    elif changed.returncode != 0:
        raise RuntimeError(changed.stderr.strip() or "failed to inspect legacy deletion")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--waap-bin", default="waap", help="waap executable used for init/check")
    parser.add_argument("--dry-run", action="store_true", help="report actions without changing files")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        repository = repository_root(Path.cwd())
        legacy = repository / ".waap"
        state = state_directory(repository)
        if not legacy.is_dir():
            raise RuntimeError(f"legacy state directory {legacy} does not exist")

        # Preflight source contents before creating or changing central state.
        source_files(legacy)
        if args.dry_run and not state.exists():
            print(f"Would initialize state directory: {state}")
            print(f"Would move {legacy} into the new state worktree")
            return 0

        initialize_state(repository, state, args.waap_bin)
        copies, conflicts = migration_plan(legacy, state)
        if conflicts:
            joined = "\n  ".join(str(path) for path in conflicts)
            raise RuntimeError(f"different files exist in both locations:\n  {joined}")

        if args.dry_run:
            print(f"State directory: {state}")
            print(f"Would copy {len(copies)} file(s) and remove {legacy}")
            return 0

        for source, destination in copies:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        commit_copies(state, [destination for _, destination in copies])

        check = run(args.waap_bin, "check", cwd=repository, check=False)
        if check.returncode:
            raise RuntimeError(f"migrated state failed waap check: {check.stdout.strip()} {check.stderr.strip()}")

        remove_legacy(repository, legacy)
        print(f"State directory: {state}")
        print(f"Migrated {len(copies)} new file(s); matching and destination-only files were preserved")
        return 0
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
        print(f"error: {detail}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
