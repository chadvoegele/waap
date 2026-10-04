# Quality-refactor validation

All commands below ran from the repository root on 2026-10-04. Filesystem
sandboxing was disabled for the session, including every `cargo test` invocation.
Toolchain: `rustc 1.99.0 (b940084d7 2026-09-28)`.

| Exact command | Result |
| --- | --- |
| `cargo clippy --all-targets -- -D warnings` | Exit 0; no warnings. |
| `cargo fmt --check` | Exit 0. |
| `cargo build` | Exit 0. |
| `cargo build --release` | Exit 0. |
| `cargo test` | Exit 0; 285 unit tests and 31 integration tests passed; none failed/ignored. |
| `python3 scripts/test_migrate_legacy_waap.py` | Exit 0; 2 tests passed. |
| `git diff --check` | Exit 0. |

Baseline before implementation: 267 unit and 26 integration tests passed. The
new CRLF parser and dependency/body characterization tests passed against the
original implementation before refactoring.

Regression coverage includes missing dependencies, self/multiple-node cycles,
20,000-node chains, shared/duplicate edges, deterministic diagnostics, unchanged
records/HEAD after cycle rejection, parallel named record allocation/commits,
concurrent opposite dependency updates, competing run ownership, atomic
publication failure/cleanup, file modes, consistent record snapshots, malformed
frontmatter, TOML control characters, local process disposal, and protocol failure.
Existing tests continue covering terminal conflicts, commit failures, worktree
cleanup errors, backend configuration precedence, HTTP/SSE contracts, repair,
scoped Git commits, and preservation of unrelated staged changes.

`tests/agent_execution.rs` runs the production CLI against a local Python stdio
protocol double. It verifies completed/failed turns, malformed startup, and failed
session publication remove and reap the Codex process before removing its
worktree. The process deliberately remains live after its response, so merely
dropping the handle would fail the assertions. These PID/zombie assertions are
Linux-specific. Tests require no real external agent or external service; existing
OpenCode component tests use a loopback HTTP server. The protocol double requires
`/usr/bin/python3` on Linux, consistent with the separate Python migration tests.

## Real-agent heat-equation workflow

Followed `.agents/skills/waap-heat-equation-e2e-test/SKILL.md` in the isolated
temporary repository:

```text
/tmp/waap-codemd-heat-nhyybs3k
```

State was explicitly isolated in that repository's `.state` worktree. The
locally built `target/debug/waap` was used and its directory prepended to PATH.
The skill's simulator specification was committed to the temporary source
repository. All waap invocations below used this prefix (where `$binary` was
the absolute path of this checkout's `target/debug/waap`):

```sh
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json
```

The planner ticket was created with `ticket new --name 'Plan heat simulator'`.
The planner instructions created one program ticket and one dependent
verification/documentation ticket. Each agent was created with its detailed
instructions on stdin using `agent new --name <name>`. The actual runs were:

```sh
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json agent run --agent-id aa-heat-planner --system codex
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json agent run --agent-id aa-heat-developer-1 --system codex
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json agent run --agent-id aa-heat-developer-2 --system codex
```

Each run exited 0. The developers integrated their committed source changes
only into the temporary repository. The real waap repository was not merged.
An independent final verification ran these exact commands in the temporary
repository; each exited 0:

```sh
python3 heat.py
python3 heat.py --ascii
python3 heat.py --verify
python3 verify_heat.py
```

The default 20x20 simulation converged in 2,530 steps. Center temperature rose
from 0 to `0.249999864`; minimum was `0.249997983`, maximum `0.250002017`, and
average `0.25`. The maximum stayed below the initial hot spot's 100. The
independent check confirmed 20 ASCII rows of 20 cells. Numerical verification
and all six generated acceptance tests passed.

Final state inspection used:

```sh
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json check
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json ticket list
"$binary" --waap-root /tmp/waap-codemd-heat-nhyybs3k/.state --output-format json agent list
git worktree list --porcelain
```

All commands exited 0. Check returned `valid: true`; all three tickets and all
three agents were completed; no managed agent worktree remained. Temporary
planner/developer logs remain in that directory for inspection. The Codex server
emitted a bubblewrap/user-namespace notice, but all runs completed under the
existing full-access configuration.

## Not verified

Rust 1.89 itself and non-Linux platforms were not exercised. No real Claude or
OpenCode backend/service was run. Real SIGKILL/power-loss recovery, subprocess
descendant cleanup, blocking-read interruption latency, concurrent init/repair
or migration, and cross-clone/distributed coordination are not claimed.
See the [architecture limits](architecture.md#failure-limits-and-recovery).
