#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("migrate-legacy-waap.py")
SPEC = importlib.util.spec_from_file_location("migrate_legacy_waap", SCRIPT)
assert SPEC and SPEC.loader
MIGRATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MIGRATE)


class MigrationPlanTest(unittest.TestCase):
    def test_merges_source_only_and_accepts_matching_and_destination_only_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "repository/.waap"
            state = root / "state"
            source_only = legacy / "agents/aa-source/agent.md"
            matching_source = legacy / "tickets/tt-shared/ticket.md"
            matching_destination = state / "tickets/tt-shared/ticket.md"
            destination_only = state / "agents/aa-destination/agent.md"
            for path, content in [
                (source_only, "source"),
                (matching_source, "matching"),
                (matching_destination, "matching"),
                (destination_only, "destination"),
            ]:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)

            copies, conflicts = MIGRATE.migration_plan(legacy, state)

            self.assertEqual(copies, [(source_only, state / source_only.relative_to(legacy))])
            self.assertEqual(conflicts, [])
            self.assertEqual(destination_only.read_text(), "destination")

    def test_rejects_different_files_at_the_same_relative_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "repository/.waap"
            state = root / "state"
            relative = Path("agents/aa-shared/agent.md")
            for base, content in [(legacy, "old"), (state, "new")]:
                path = base / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)

            copies, conflicts = MIGRATE.migration_plan(legacy, state)

            self.assertEqual(copies, [])
            self.assertEqual(conflicts, [relative])


if __name__ == "__main__":
    unittest.main()
