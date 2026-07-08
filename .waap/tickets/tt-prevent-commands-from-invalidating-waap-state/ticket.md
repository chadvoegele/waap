+++
name = "Prevent commands from invalidating waap state"
creation_date = 2026-07-08T11:58:15Z
status = "pending"
+++

## Context

`src/app.rs` runs `check_waap(waap_root)` before dispatching `agent` and `ticket` commands, but individual commands can still make previously-valid `.waap/` state invalid if they accept inconsistent inputs.

One current example is ticket dependencies:

- `ticket new --depends-on tt-missing` validates only that `tt-missing` is syntactically a ticket id.
- `ticket update --add-depends-on tt-missing` has the same issue.
- `waap check` later reports the missing dependency, but the successful mutation already wrote invalid state.

We want a stronger command invariant: successful waap mutation commands should not invalidate waap state.

This ticket implements that invariant for the known ticket-dependency gap. Do not broaden it into an audit or rewrite of every mutation command.

## Proposed Spec Change

Update `specs/spec.md` to state that waap mutation commands must not leave `.waap/` invalid when they return success.

Clarify that manual edits can still make state invalid and `waap check` remains the validator for manual state repair.

For ticket dependencies, specify that:

- `ticket new --depends-on <ticket-id>` requires each dependency to be an existing ticket.
- `ticket update --add-depends-on <ticket-id>` requires each added dependency to be an existing ticket.
- `ticket update --remove-depends-on <ticket-id>` may remain a no-op for dependencies that are not currently present, after syntax validation, because removing a missing dependency does not invalidate state.

## Proposed Implementation

Add a reusable ticket metadata loader, likely near `load_ticket_metadata()` in `src/ticket.rs`:

```rust
pub(crate) fn load_tickets_metadata(waap_root: &Path) -> io::Result<Vec<TicketMetadata>> {
    let ticket_ids = list_record_ids(waap_root, WaapRecordKind::Ticket)?;
    ticket_ids
        .iter()
        .map(|ticket_id| load_ticket_metadata(waap_root, ticket_id))
        .collect()
}
```

Use the shared loader to build the set of existing ticket IDs.

In `src/ticket/new.rs`:

- Keep current syntax validation with `is_ticket_id()`.
- Add existence validation before writing the new ticket.
- Reject syntactically-valid missing dependencies with an `InvalidInput` error.

In `src/ticket/update.rs`:

- Keep syntax validation for both `--add-depends-on` and `--remove-depends-on`.
- Validate existence only for `--add-depends-on` values.
- Do not require removed dependency IDs to exist as tickets.

Avoid over-generalizing beyond current callers. `check_tickets()` may keep its current error-accumulating logic; the new loader can fail fast for command execution.

## Tests

Update tests so dependency-using ticket creation creates dependency tickets first.

Add or update tests for:

- `ticket new` accepts dependencies that are valid existing tickets.
- `ticket new` rejects syntactically invalid dependency IDs.
- `ticket new` rejects syntactically valid but missing dependency IDs.
- `ticket update --add-depends-on` accepts existing ticket IDs.
- `ticket update --add-depends-on` rejects syntactically valid but missing ticket IDs.
- `ticket update --remove-depends-on` remains a no-op when the dependency is not present.

## Validation

Run from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
