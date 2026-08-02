# Central waap state worktree

## Summary

Store each repository's waap state in one dedicated Git worktree instead of
copying `.waap` into every application worktree. The dedicated worktree uses a
local orphan branch named `waap` and lives below `~/.local/state/waap/`.

For example, the repository at:

```text
/home/chad/code/github.com/chadvoegele/waap
```

uses:

```text
~/.local/state/waap/home/chad/code/github.com/chadvoegele/waap/ # worktree and state directory
```

This gives all `waap` commands launched from the primary checkout or any linked
worktree one state checkout, index, branch, and history.

## Goals

- Resolve the same state from every worktree belonging to a repository.
- Keep waap state commits out of application branch lineages.
- Preserve waap state as plain files tracked by Git.
- Migrate repositories that currently track `.waap` on application branches.
- Report the resolved state directory through `waap check`.
- Continue validating direct state edits with `waap check`.

## Review-sensitive decisions

This proposal intentionally chooses the following behaviors for review:

- The canonical primary checkout path, not the remote URL, identifies a clone.
- Migration commits removal of the unique legacy `.waap` on its application
  branch.
- If central state and legacy `.waap` coexist, waap returns an error instead
  of choosing or merging state.
- `waap check` fails when central state has staged, unstaged, or untracked
  changes, even when their contents are otherwise valid.
- The local `waap` branch tracks `origin/waap`; initialization configures the
  upstream but does not push it.
- `waap check` fetches `origin/waap` and fails when the remote contains commits
  that are not in the local `waap` branch.

## Non-goals

- Synchronizing state between separate clones or hosts.
- Automatically pushing the `waap` branch or fetching it outside
  `waap check`.
- Automatically reconciling central state and legacy `.waap` that coexist.
- Preserving support for more than one waap project in a Git repository.

## Terminology

- **Invocation worktree**: the checkout containing the caller's current
  directory. It is normally an application checkout, but may be the state
  worktree when the caller runs there.
- **State override**: the waap state directory explicitly supplied through
  `--waap-root`.
- **Primary repository root**: the canonical root whose `.git` directory is the
  common Git directory. This path identifies the repository.
- **State worktree**: the checkout below `~/.local/state/waap/` on branch
  `waap`.
- **State directory**: the state worktree root. Its `agents` and `tickets`
  directories are tracked directly, without a `.waap` wrapper.
- **Legacy state**: a `.waap` entry in the invocation worktree rather than the
  state worktree.

`--waap-root` always names the state directory containing `agents` and
`tickets`, never an application checkout. In a migrated project this is the
central state worktree root; in a legacy project it is the application's
`.waap` directory. Without it, waap derives the central state directory from
the invocation application's common Git directory. The current CLI can
initialize nested projects in one repository by pointing `--waap-root` at
different application directories. This proposal intentionally changes that
behavior: one common Git directory identifies one waap project.

## Repository and state resolution

Resolution must not depend on the current application branch or on finding a
`.waap` ancestor.

1. Without `--waap-root`, canonicalize the current directory and walk upward
   to its nearest `.git` entry. This is the invocation worktree root. Resolve
   its common Git directory and derive the state directory below
   `~/.local/state/waap`.
2. With `--waap-root`, canonicalize the supplied state directory instead of
   using the current directory. Require it to contain `agents` and `tickets`,
   then resolve the common Git directory by walking upward from it. After
   migration, the supplied directory must equal the path derived below
   `~/.local/state/waap` for that Git repository.
3. A `.git` directory is the common directory. For a `.git` file, resolve its
   `gitdir:` target and its `commondir` file. This handles linked worktrees.
4. Require the common directory to be `<primary repository root>/.git` and
   canonicalize the primary repository root. Unsupported bare repositories or
   separate-Git-dir layouts fail with a specific error.
5. Remove the leading root separator from the primary repository path and
   append the remaining components to `~/.local/state/waap`. The resulting path
   is both the state worktree and state directory.

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
`waap agent run` creates an agent worktree from an application invocation
worktree's HEAD, not from the orphan `waap` branch. When `--waap-root` is used,
source operations separately resolve the application invocation worktree from
the current directory and reject the state worktree because no source HEAD was
selected.

### Repository relocation

Git records absolute links between the common Git directory and linked
worktrees. Moving the primary repository therefore breaks the state worktree's
back-link until `git worktree repair` updates it. The move also changes waap's
path-derived state location.

From the moved primary repository, `waap check` detects the registered `waap`
worktree at its old location, reports the newly expected state directory, and
instructs the caller to run `waap init`. Initialization repairs the Git linkage,
moves the state worktree to the newly resolved path, and repairs the
registration. It preserves the branch, state, dirty files, and commit history.
It fails without moving anything if the destination is occupied or the
registered worktree cannot be identified safely.

A caller in another linked worktree whose `.git` file was also broken by the
primary move must first run Git's repair procedure from the primary repository.

## State branch and worktree invariants

An initialized project satisfies all of these conditions:

1. `refs/heads/waap` exists.
2. The branch has no merge base with any application branch. Its first commit
   has no parents.
3. The expected state worktree is registered with Git and checks out `waap`.
4. Branch `waap` has remote `origin` and merge ref `refs/heads/waap`, making
   `origin/waap` its upstream even before the remote ref's first push.
5. The expected state worktree contains `agents` and `tickets` at its root.
6. Only waap state is tracked on the branch. The worktree does not contain an
   application source checkout.
7. Git commits made by waap stage only explicit state paths and are created on
   `waap`.
8. The state worktree has no staged, unstaged, or untracked changes.
9. When `origin/waap` exists, all of its commits are reachable from local
   `waap`.

The local branch name is always `waap`; it is not configurable. Initialization
sets `branch.waap.remote = origin` and
`branch.waap.merge = refs/heads/waap`. A plain `git push` from the state
worktree therefore creates or updates `origin/waap`. Waap does not automatically
push the branch. `waap check` fetches the remote branch only to validate its
relationship to local state. The local branch ref prevents its commits from
being garbage-collected.

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
5. Configures `origin/waap` as the branch upstream.

Initialization leaves the application branch and working tree unchanged.

Every successful `waap init` report includes the resolved absolute state
directory. Human-readable output includes:

```text
State directory: /home/chad/.local/state/waap/home/chad/code/github.com/chadvoegele/waap
```

JSON output adds a `state_directory` string to the existing init report. This
applies to fresh initialization, legacy migration, and an already initialized
project.

### Legacy migration

Every command determines whether the central state directory and legacy
`.waap` in the invocation worktree exist.

- With legacy `.waap` and no central state, commands other than `waap init`
  exit unsuccessfully and instruct the caller to initialize.
- With both central state and legacy `.waap`, every command, including
  `waap init`, returns an error listing both paths. Waap does not choose,
  compare, or merge them.
- `waap check` reports the expected central state directory before reporting
  either error.

With legacy `.waap` and no central state, the invocation worktree must have no
staged or unstaged changes outside `.waap` and no unresolved conflicts. Dirty
legacy state is allowed; migration copies its working-tree contents. `waap init`
then:

1. Validates the legacy state without modifying either checkout.
2. Creates the orphan state branch and worktree.
3. Copies the contents of legacy `.waap` into the central worktree root.
4. Validates and commits central state with subject `waap migrate state`.
5. Configures `origin/waap` as the branch upstream.
6. Removes legacy `.waap` and commits that deletion on its application branch
   with subject `Remove legacy waap state`.

The central commit happens before source removal so a partial failure does not
lose state. If source cleanup fails, later commands report central and legacy
state together; the error instructs the user to remove the legacy copy
manually. Waap never deletes legacy state until central state is valid and
committed.

When central state exists and no legacy state exists, `waap init` repairs a
missing or incorrect `origin/waap` upstream, then reports that the repository is
already initialized.

## Command behavior

After resolution, all state-aware commands operate on the central state
worktree. Mutation reports continue returning commit hashes, but those hashes
now belong to branch `waap`.

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

- the branch, upstream, and registered-worktree invariants above;
- that legacy `.waap` does not coexist in the invocation worktree;
- the state root directories, frontmatter, ID, status, and dependency
  invariants;
- the contents of files currently present in the state worktree;
- that the state worktree has no staged, unstaged, or untracked changes; and
- that fetched `origin/waap` is not ahead of local `waap`.

To check the remote relationship, `waap check` queries `origin` and fetches
`origin/waap` when that ref exists. A missing remote branch is valid before the
first push. A query or fetch failure makes the check fail because waap cannot
establish whether local state is current.

After fetching, the check fails when `git rev-list waap..origin/waap` contains
any commit. This includes both a strictly-ahead remote and diverged branches.
The error reports the number of remote-only commits and instructs the user to
integrate remote state before retrying. A remote behind local `waap` does not
fail the check.

The command always reports the resolved absolute state directory, including
when state is absent, legacy migration is required, or worktree repair is
needed. Human-readable output starts with the path:

```text
State directory: /home/chad/.local/state/waap/home/chad/code/github.com/chadvoegele/waap
OK: waap state is valid
```

JSON output adds `state_directory` to the existing check result:

```json
{
  "state_directory": "/home/chad/.local/state/waap/home/chad/code/github.com/chadvoegele/waap",
  "valid": true,
  "errors": []
}
```

`waap check` never stages or commits direct edits. Any dirty state makes the
check fail, even when its contents pass structural validation. The error lists
the changed paths and requires the user to revert them or commit them on
`waap` before retrying. This makes bypassing the CLI visible while preserving
plain files as a recovery mechanism.

## Failure and recovery

Commands must fail without modifying state when:

- the caller is outside a supported non-bare Git repository;
- remote `origin` is missing;
- `HOME` cannot produce the required absolute state path;
- the expected path is occupied by an unrelated file or checkout;
- branch `waap` exists but is not an orphan waap state branch;
- `waap` is checked out in another location and cannot be safely relocated;
- the registered state worktree, branch, or common Git directory disagrees
  with the resolved repository;
- central state and legacy `.waap` coexist, or legacy state is invalid;
- `waap check` cannot query or fetch `origin/waap`, or finds it ahead of local
  `waap`; or
- the state mutation lock cannot be acquired.

Errors include the conflicting path or ref and a recovery action. Waap must not
silently recreate missing state when the `waap` branch still exists. Recovery
uses `git worktree repair` or a subsequent `waap init`; destructive reset is
always explicit.

## Documentation changes

Implementation must update the README, main specification, waap skill, and
agent role templates so that:

- central state paths use the state directory while `.waap` refers only to
  legacy state in an application checkout;
- examples use the CLI for mutations;
- direct edits are followed by `waap check`, which reports uncommitted state as
  an error; and
- agent worktree instructions distinguish source worktrees from the state
  worktree.

## Acceptance criteria

1. Commands from the primary checkout and two linked worktrees report the same
   state directory and observe each other's state commits immediately.
2. After initialization or migration, state mutations change only `waap`;
   application branch HEADs do not move.
3. `waap` has no merge base with `main` after fresh initialization and tracks
   `origin/waap`; a plain `git push` from the state worktree pushes that ref.
4. Agent source worktrees start from the invoking application HEAD, never from
   `waap`.
5. A repository with tracked legacy `.waap` is blocked until `waap init`
   migrates and removes it without data loss.
6. Central state plus legacy `.waap` in the invocation worktree produces an
   error listing both paths and changes no checkout.
7. `waap init` and `waap check` report the resolved absolute state directory in
   human-readable and JSON formats.
8. Staged, unstaged, and untracked direct edits fail `waap check`, including
   edits whose contents are otherwise valid.
9. `waap check` passes when `origin/waap` is absent or behind local state, and
   fails after fetching when the remote is ahead or diverged.
10. A pre-existing non-orphan `waap` branch is preserved and causes a useful
    error.
11. Concurrent mutation attempts serialize or fail cleanly without leaving
    invalid or partially committed state.
12. Moving the primary repository makes `waap check` report the newly expected
    state directory and old registered path; `waap init` relocates and repairs
    the state worktree without changing its state or history.
