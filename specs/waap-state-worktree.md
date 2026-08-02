# Central waap state worktree

## Summary

Store each repository's waap state in one dedicated Git worktree instead of
copying `.waap` into every application worktree. The dedicated worktree uses a
local orphan branch named `waap` and lives below `~/.waap/state/`.

For example, the repository at:

```text
/home/chad/code/github.com/chadvoegele/waap
```

uses:

```text
~/.waap/state/home/chad/code/github.com/chadvoegele/waap/ # worktree and state directory
```

This gives all `waap` commands launched from the primary checkout or any linked
worktree one state checkout, index, branch, and history.

## Goals

- Resolve the same state from every worktree belonging to a repository.
- Keep waap state commits out of application branch lineages.
- Preserve waap state as plain files tracked by Git.
- Migrate repositories that currently track `.waap` on application branches.
- Expose the resolved paths through `waap status`.
- Continue validating direct state edits with `waap check`.

## Review-sensitive decisions

This proposal intentionally chooses the following behaviors for review:

- The canonical primary checkout path, not the remote URL, identifies a clone.
- Migration commits removal of legacy `.waap` files on the invoking application
  branch.
- If another application branch still contains legacy `.waap` after the first
  migration, migrating that branch merges only disjoint or byte-identical
  files; differing files require manual reconciliation.
- Valid direct edits may remain uncommitted after `waap check`.
- Waap never pushes the local state branch.

## Non-goals

- Synchronizing state between separate clones or hosts.
- Automatically pushing or fetching the `waap` branch.
- Combining conflicting legacy state without user review.
- Preserving support for more than one waap project in a Git repository.

## Terminology

- **Invocation worktree**: the application checkout containing the caller's
  current directory or `--waap-root`.
- **Primary repository root**: the canonical root whose `.git` directory is the
  common Git directory. This path identifies the repository.
- **State worktree**: the checkout below `~/.waap/state/` on branch `waap`.
- **State directory**: the state worktree root. Its `agents` and `tickets`
  directories are tracked directly, without a `.waap` wrapper.
- **Legacy state**: a `.waap` entry in an invocation worktree rather than the
  state worktree.

`--waap-root` continues to select an application checkout. It does not override
where state is stored. The current CLI can initialize nested projects in one
repository by pointing `--waap-root` at different directories. This proposal
intentionally changes that behavior: one common Git directory identifies one
waap project.

## Repository and state resolution

Resolution must not depend on the current application branch or on finding a
`.waap` ancestor.

1. Canonicalize the current directory, or `--waap-root` when supplied.
2. Walk upward to the nearest `.git` entry. This is the invocation worktree
   root. Error if no `.git` entry exists.
3. Resolve the common Git directory:
   - A `.git` directory is already the common directory.
   - For a `.git` file, resolve its `gitdir:` target and its `commondir` file.
     This handles linked worktrees.
4. Require the common directory to be `<primary repository root>/.git` and
   canonicalize the primary repository root. Unsupported bare repositories or
   separate-Git-dir layouts fail with a specific error.
5. Remove the leading root separator from the primary repository path and
   append the remaining components to `~/.waap/state`. The resulting path is
   both the state worktree and state directory.

Symlinks are resolved before deriving the path. Two clones at different paths
therefore have independent state. `HOME` must be set and absolute.

Resolution should produce one context rather than reuse one path for unrelated
purposes:

```text
ProjectContext
  invocation_worktree_root
  primary_repository_root
  common_git_dir
  state_root
```

State reads, writes, validation, staging, and commits use the state worktree.
Application source operations use the invocation worktree. In particular,
`waap agent run` creates an agent worktree from the invocation worktree's HEAD,
not from the orphan `waap` branch.

### Repository relocation

Git records absolute links between the common Git directory and linked
worktrees. Moving the primary repository therefore breaks the state worktree's
back-link until `git worktree repair` updates it. The move also changes waap's
path-derived state location.

From the moved primary repository, `waap status` detects the registered `waap`
worktree at its old location, reports the newly expected path, and instructs the
caller to run `waap init`. Initialization repairs the Git linkage, moves the
state worktree to the newly resolved path, and repairs the registration. It
preserves the branch, state, dirty files, and commit history. It fails without
moving anything if the destination is occupied or the registered worktree
cannot be identified safely.

A caller in another linked worktree whose `.git` file was also broken by the
primary move must first run Git's repair procedure from the primary repository.

## State branch and worktree invariants

An initialized project satisfies all of these conditions:

1. `refs/heads/waap` exists.
2. The branch has no merge base with any application branch. Its first commit
   has no parents.
3. The expected state worktree is registered with Git and checks out `waap`.
4. The expected state worktree contains `agents` and `tickets` at its root.
5. Only waap state is tracked on the branch. The worktree does not contain an
   application source checkout.
6. Git commits made by waap stage only explicit state paths and are created on
   `waap`.

The local branch name is always `waap`; it is not configurable. Waap does not
push it. The branch ref prevents its commits from being garbage-collected.

A branch named `waap` that has application ancestry is not adopted or reset.
Initialization fails without changing it and explains how to rename or remove
the conflicting branch.

## `waap init`

### New repository

When neither central nor legacy state exists, `waap init`:

1. Creates the state worktree's parent directories.
2. Creates orphan branch `waap` and its worktree at the resolved path.
3. Creates the `agents` and `tickets` skeleton at the worktree root.
4. Creates a parentless `waap init` commit on `waap`.
5. Adds the invocation-root `/.waap/` path to the repository's local exclude
   file so accidental state is not added to application branches.

Initialization leaves the application branch and working tree unchanged.

### Legacy migration

Every command except `waap init` first checks for legacy state. If the
invocation worktree contains `.waap`, the command exits unsuccessfully and
instructs the caller to run `waap init`. `waap status` still prints the expected
central paths and reports `migration_required = true` before returning failure.

The invocation worktree must have no staged or unstaged changes outside
`.waap`, and must have no unresolved conflicts. Dirty legacy state is allowed;
the migration copies its working-tree contents. `waap init` then:

1. Validates the legacy state without modifying either checkout.
2. Creates the orphan state branch and worktree if needed.
3. Builds a candidate state by copying the contents of legacy `.waap` into the
   central worktree root. Existing byte-identical paths are accepted and
   central-only paths are retained.
4. If the same relative path differs, aborts before copying anything and lists
   every conflict. The user resolves each conflict in either checkout and
   retries `waap init`. To validate a direct central edit first, run
   `waap check` from the state worktree so the legacy application checkout is
   not the invocation worktree.
5. Validates the complete candidate state.
6. Commits the migrated state on `waap` with subject `waap migrate state`.
7. Removes `.waap` from the invocation worktree and commits that deletion on
   its application branch with subject `Remove legacy waap state`.
8. Adds `/.waap/` to the repository's local exclude file.

The central commit happens before source removal so a partial failure does not
lose state. Initialization is retryable: if central files already match the
legacy files, it continues with source removal. It never deletes legacy state
until central state is valid and committed.

An application branch that does not contain the deletion commit may expose its
legacy `.waap` when checked out later. Running `waap init` from that branch
migrates it too: disjoint files are added to central state and conflicting files
require explicit review. Branches containing the deletion commit through merge
or rebase need no later migration.

When central state exists and the invocation worktree has no legacy state,
`waap init` reports that the repository is already initialized.

## Command behavior

After resolution, all agent, ticket, check, and status commands operate on the
central state directory. Mutation reports continue returning commit hashes,
but those hashes now belong to branch `waap`.

Mutation transactions must be serialized with one per-repository lock outside
the state worktree, such as in the common Git directory. A transaction holds
the lock across pre-validation, file updates, post-validation, staging, and
commit. `waap agent run` does not hold this lock while the external agent runs;
each state transition is a separate transaction.

Agent instructions and bundled role templates must tell agents to use the
`waap` CLI for state changes. Runner prompts may include the absolute resolved
path to `agent.md` for reading, but must not instruct agents to edit it. An agent
that intentionally edits central files directly must run `waap check`
afterward.

## `waap check`

`waap check` validates:

- the branch and registered-worktree invariants above;
- that no legacy `.waap` exists in the invocation worktree;
- the state root directories, frontmatter, ID, status, and dependency
  invariants; and
- the files currently present in the state worktree, including uncommitted
  direct edits.

A successful check does not stage or commit direct edits. Dirty but valid state
is valid; `waap status` exposes whether it is dirty. This preserves the plain
file escape hatch while making the CLI the preferred mutation interface.

## `waap status`

Add a top-level, read-only `waap status` command. It resolves paths even when
state is absent or migration is required.

Human-readable output includes at least:

```text
Repository: /home/chad/code/github.com/chadvoegele/waap
Invocation worktree: /home/chad/code/github.com/chadvoegele/waap/worktrees/example
State worktree: /home/chad/.waap/state/home/chad/code/github.com/chadvoegele/waap
State directory: /home/chad/.waap/state/home/chad/code/github.com/chadvoegele/waap
State branch: waap
Initialized: true
Migration required: false
State clean: true
```

JSON output uses stable absolute path strings:

```json
{
  "repository_root": "/home/chad/code/github.com/chadvoegele/waap",
  "invocation_worktree": "/home/chad/code/github.com/chadvoegele/waap/worktrees/example",
  "state_worktree": "/home/chad/.waap/state/home/chad/code/github.com/chadvoegele/waap",
  "state_directory": "/home/chad/.waap/state/home/chad/code/github.com/chadvoegele/waap",
  "state_branch": "waap",
  "initialized": true,
  "migration_required": false,
  "state_clean": true
}
```

`state_clean` is `null` when the state worktree is unavailable. Status exits
successfully only when initialized, no migration is required, and the topology
is usable. Otherwise it prints the report, emits `waap init` or repair guidance,
and exits unsuccessfully.

## Failure and recovery

Commands must fail without modifying state when:

- the caller is outside a supported non-bare Git repository;
- `HOME` cannot produce the required absolute state path;
- the expected path is occupied by an unrelated file or checkout;
- branch `waap` exists but is not an orphan waap state branch;
- `waap` is checked out in another location and cannot be safely relocated;
- the registered state worktree, branch, or common Git directory disagrees
  with the resolved repository;
- legacy migration has conflicting files or invalid state; or
- the state mutation lock cannot be acquired.

Errors include the conflicting path or ref and a recovery action. Waap must not
silently recreate missing state when the `waap` branch still exists. Recovery
uses `git worktree repair` or a subsequent `waap init`; destructive reset is
always explicit.

## Documentation changes

Implementation must update the README, main specification, waap skill, and
agent role templates so that:

- central state paths use the state worktree root while `.waap` refers only to
  legacy state in an application checkout;
- examples use the CLI for mutations;
- direct edits are followed by `waap check`; and
- agent worktree instructions distinguish source worktrees from the state
  worktree.

## Acceptance criteria

1. Commands from the primary checkout and two linked worktrees report the same
   state directory and observe each other's state commits immediately.
2. After initialization or migration, state mutations change only `waap`;
   application branch HEADs do not move.
3. `waap` has no merge base with `main` after fresh initialization.
4. Agent source worktrees start from the invoking application HEAD, never from
   `waap`.
5. A repository with tracked legacy `.waap` is blocked until `waap init`
   migrates and removes it without data loss.
6. Conflicting migration from another legacy application branch lists
   conflicts and changes neither checkout.
7. `waap status` reports resolved absolute paths in human-readable and JSON
   formats before and after initialization.
8. Direct valid edits pass `waap check` and make `state_clean` false; invalid
   edits fail `waap check`.
9. A pre-existing non-orphan `waap` branch is preserved and causes a useful
   error.
10. Concurrent mutation attempts serialize or fail cleanly without leaving
    invalid or partially committed state.
11. Moving the primary repository makes `waap status` report the old and new
    state paths; `waap init` relocates and repairs the state worktree without
    changing its state or history.
