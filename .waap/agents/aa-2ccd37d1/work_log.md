# Work Log — aa-2ccd37d1 / tt-add-waap-init-command-require-waap-for-mutating-commands

## Plan
- Add `git::is_inside_git_work_tree` helper.
- New `src/init.rs`: `init_project`, `InitReport { path, marker }`, `print_init_report`.
  - `.waap` already exists -> error.
  - repo_root not inside git work tree -> error.
  - creates `.waap/agents`, `.waap/tickets`, and `.waap/.gitkeep` (marker file so the otherwise-empty
    skeleton has something to commit; check_waap only inspects agents/tickets subdirs, so a file
    directly under `.waap/` doesn't trip its validation).
  - report.path = canonicalized repo_root (the "initialized root").
- `record::require_initialized_project(repo_root)` — checks `.waap` is a dir, else
  `io::ErrorKind::NotFound` with message `no waap project found; run 'waap init'` (exact wording
  requested by the ticket for compatibility with the follow-up root-resolution ticket).
- Call `require_initialized_project` at the top of `create_ticket_with_markdown` and
  `create_agent_with_markdown` so `ticket new` / `agent new` no longer implicitly create `.waap`.
- cli.rs: add `Command::Init` (no extra flags; reuses the existing global `--repo-root`).
- app.rs: wire `Command::Init` through `commit_and_print` like other mutating commands.
- Existing tests in `ticket/new.rs` / `agent/new.rs` create a tempdir with no `.waap` and call
  `create_*_with_markdown` directly — need to `fs::create_dir_all(dir.path().join(".waap"))` first
  now that the implicit-create behavior is gone.
- Add new tests: init happy path, init when `.waap` exists, init outside git, ticket/agent new
  erroring when uninitialized.
