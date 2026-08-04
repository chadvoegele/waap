# waap

`waap` is an agent automation platform for planning work as tickets, running disposable agents, and keeping their state in Git.

<img width="512" height="512" alt="waap" src="https://github.com/user-attachments/assets/929a5b2c-6c08-457a-b16f-d00c34bfa080" />

## State model

Each application repository has one central waap state worktree on the local orphan branch `waap`. For a primary checkout at:

```text
/home/chad/code/github.com/chadvoegele/example
```

its state directory is:

```text
~/.local/state/waap/data/home/chad/code/github.com/chadvoegele/example
```

The state directory contains `agents/` and `tickets/` directly. It is separate from application source worktrees, so state commits stay on `waap` and never move an application branch. The path is derived from the primary checkout, so the primary checkout and every linked source worktree use the same state immediately.

`.waap` means only legacy state inside an application checkout. A project with legacy state must be migrated with `waap repair`; do not copy it into source worktrees.

Run this once in a new Git repository:

```sh
waap init
```

`waap init` is setup-only: it creates or adopts central state and reports its absolute state directory. It refuses existing central or legacy state. Use `waap repair` to migrate legacy state, repair a moved state worktree, or restore `origin/waap` tracking.

```sh
waap check
```

`waap init` and `waap check` report the resolved absolute state directory in human-readable output and as `state_directory` in JSON. `waap check` also validates its Git and record invariants.

## Workflow

Create a ticket and an agent with the CLI:

```sh
printf 'Implement the file picker' | waap ticket new --name 'File Picker'
printf 'Implement tt-file-picker.' | waap agent new --name 'File Picker Implementer'
waap agent run --agent-id aa-file-picker-implementer
```

Tickets live at `$WAAP_STATE/tickets/<ticket-id>/ticket.md`; agents live at `$WAAP_STATE/agents/<agent-id>/agent.md`, where `$WAAP_STATE` is the directory reported by `waap init` or `waap check`. Records use TOML frontmatter and Markdown bodies. Tickets may depend on other tickets to form a DAG.

`waap agent run` creates a temporary **source** worktree from the invoking application's HEAD, runs the agent there, and removes it afterward. It never creates a source worktree from `waap`. The launcher owns this lifecycle; agents must not create or remove source worktrees themselves.

## State safety

Use `waap` commands for all normal state mutations (`ticket new`/`update`, `agent new`/`run`/`update`/`stop`). They serialize validation, writes, and commits on `waap`.

Plain files remain available for emergency recovery. If a direct edit is unavoidable:

1. Edit the central state directory, not an application checkout.
2. Run `waap check`; its dirty-state failure is expected while the edit is uncommitted. Fix every reported content error.
3. Explicitly commit the validated files on the `waap` branch in the state worktree.
4. Run `waap check` again. It must pass cleanly.

`waap check` never stages, commits, or repairs direct edits.

## State location and repair

`--waap-root` always identifies a **state directory** containing `agents` and `tickets`, never an application checkout. For commands other than `init`, it uses that directory directly without deriving, relocating, or comparing state. For `waap init --waap-root <path>`, `<path>` is the exact new state-directory target.

Without `--waap-root`, waap derives the central state directory from the current source worktree's primary checkout. For example:

```sh
waap --waap-root /srv/waap-state ticket list
waap init --waap-root /srv/new-waap-state
```

If a primary repository moves, run `waap repair` from its primary checkout. If central and legacy state coexist, reconcile them manually before repairing; waap will not merge or choose between them.

## Remotes

When `origin` exists, `waap init` configures local `waap` to track `origin/waap`, but does not push. State synchronization is manual:

```sh
cd "$WAAP_STATE"
git push
```

`waap check` never accesses the network. When a cached `origin/waap` ref exists, it warns if that ref is ahead or diverged. Fetch state explicitly before checking remote freshness, then deliberately update or rebase before pushing.

## CLI and skill

For the complete command reference:

```sh
waap --help
waap ticket --help
waap agent --help
```

The bundled skill and role templates are:

- `.agents/skills/waap/SKILL.md`
- `.agents/skills/waap/roles/planner/agent.md`
- `.agents/skills/waap/roles/developer/agent.md`

## Build

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
