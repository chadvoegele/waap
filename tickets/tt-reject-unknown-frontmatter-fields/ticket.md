+++
title = "Reject unknown frontmatter fields"
creation_date = 2026-06-27T02:22:11Z
status = "completed"
+++

# Problem

waap accepted planner-created ticket frontmatter containing `dependencies = [...]`, even though the supported field is `depends_on = [...]`. Because the unknown field was ignored, `waap check` passed and `waap ticket list --unblocked` treated dependent tickets as unblocked.

# Desired Behavior

Ticket and agent frontmatter should be strictly validated. Optional known fields such as `depends_on` should remain optional, but unknown fields must be rejected by `waap check` and by record loading paths.

# Acceptance Criteria

1. Ticket frontmatter rejects unknown fields such as `dependencies`.
2. Agent frontmatter rejects unknown fields outside the supported agent schema.
3. `depends_on` remains optional.
4. When `depends_on` is present, each dependency id is validated, must exist during `waap check`, and participates in blocked/unblocked filtering.
5. `waap check` fails with a clear error message naming the unknown field and record path.
6. Tests cover unknown ticket fields, unknown agent fields, valid missing `depends_on`, and valid present `depends_on`.
