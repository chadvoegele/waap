+++
title = "waap ticket new --depends-on flag"
creation_date = 2026-06-26T21:52:38Z
status = "pending"
depends_on = ["tt-add-dependson-to-ticket-schema"]
+++

# Spec Reference
specs/ticket_dependencies.md — CLI Changes / waap ticket new

# Description
Add a repeatable `--depends-on` flag to `waap ticket new` so dependencies can be declared at creation time.

# Requirements
- Add `--depends-on <TICKET_ID>` as a repeatable optional parameter in the `TicketCommand::New` variant in `src/cli.rs`.
- Pass the collected IDs through to `create_ticket()` in `src/ticket/new.rs`.
- Store them in the `TicketMetadata` written to disk.
- Validate each supplied ID is well-formed (matches `is_ticket_id()`) before writing; return a clear error if not.
- Add a test: create a ticket with `--depends-on`, read it back, confirm the field round-trips.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
