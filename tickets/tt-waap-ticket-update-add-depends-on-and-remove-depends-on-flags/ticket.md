+++
title = "waap ticket update --add-depends-on and --remove-depends-on flags"
creation_date = 2026-06-26T21:52:46Z
status = "completed"
depends_on = ["tt-add-dependson-to-ticket-schema"]
+++

# Spec Reference
specs/ticket_dependencies.md — CLI Changes / waap ticket update

# Description
Add `--add-depends-on` and `--remove-depends-on` flags to `waap ticket update` so dependencies can be modified on existing tickets.

# Requirements
- Add `--add-depends-on <TICKET_ID>` (repeatable) and `--remove-depends-on <TICKET_ID>` (repeatable) to `TicketCommand::Update` in `src/cli.rs`.
- At least one of `--set-status`, `--add-depends-on`, or `--remove-depends-on` must be provided (extend existing "at least one" validation).
- In `src/ticket/update.rs`, load the existing `depends_on` list, apply additions and removals, and write the updated metadata back.
- Removing an ID that isn't present is a no-op (not an error).
- Validate each supplied ID is well-formed before applying changes.
- Add tests: add a dep, remove a dep, add-and-remove in one call, remove a non-existent dep (no error).

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
