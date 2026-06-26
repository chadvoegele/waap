# Ticket Dependencies

## Overview

Tickets may declare dependencies on other tickets. A ticket with unsatisfied dependencies is considered **blocked**. A ticket whose dependencies are all completed is **unblocked**.

Dependencies are expressed as a `depends_on` field in ticket TOML frontmatter, which is a list of ticket IDs. Waap tooling validates and surfaces these relationships.

## Schema

Add an optional `depends_on` field to the ticket schema, which is a list of ticket IDs. If it's empty or missing, there are no dependencies.

```
+++
title = "Frontend import UI"
creation_date = 2026-06-26T20:31:07Z
status = "pending"
depends_on = ["tt-mlflow-backend-proxy-endpoints"]
+++
```

Each entry must be a valid ticket ID.

## Validation (`waap check`)

`waap check` validates:

1. Each entry in `depends_on` is a well-formed ticket ID.
2. Each referenced ticket ID exists in `.waap/tickets/`.
3. The dependency graph contains no cycles (detected via depth-first search across all tickets).

## CLI Changes

### `waap ticket new`

Add an optional `--depends-on` flag (repeatable) to declare dependencies:

```
waap ticket new --title "Frontend MLflow import UI" --depends-on tt-mlflow-backend-proxy-endpoints
```

Multiple dependencies:

```
waap ticket new --title "Deploy pipeline" \
  --depends-on tt-build-artifacts \
  --depends-on tt-integration-tests
```

### `waap ticket update`

Add an optional `--add-depends-on` and `--remove-depends-on` flag to modify dependencies:

```
waap ticket update --ticket-id tt-frontend-mlflow-import-ui --add-depends-on tt-auth-middleware
waap ticket update --ticket-id tt-frontend-mlflow-import-ui --remove-depends-on tt-mlflow-backend-proxy-endpoints
```

### `waap ticket list`

Add an optional `--blocked` / `--unblocked` filter to return only tickets that are blocked or unblocked by their dependencies:

```
waap ticket list --unblocked --status pending
```

A ticket is **unblocked** when all tickets in its `depends_on` list have status `completed`. A ticket with no dependencies is always unblocked.

The human-readable output of `waap ticket list` should annotate each ticket with its blocked/unblocked state.
