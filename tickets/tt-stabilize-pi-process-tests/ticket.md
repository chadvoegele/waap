+++
name = "Stabilize Pi process tests"
creation_date = 2026-08-07T17:58:16Z
status = "completed"
depends_on = ["tt-pi-backend-integration-verification"]
+++

# Stabilize Pi fake-process protocol tests

The post-loop independent validation of PR #6 at `deb3821` exposed a flaky failure:

```text
agent::pi::tests::prompt_rejection_malformed_json_timeout_and_eof_fail_closed
left: ExecutableFileBusy
right: Other
```

## Scope

- Reproduce and identify the process/script lifecycle race rather than weakening assertions.
- Fix production child cleanup or test-fixture executable creation as appropriate.
- Ensure every fake Pi child is closed and reaped and executable fixtures cannot race with execution.
- Audit neighboring Pi process tests for the same lifecycle issue.
- Preserve the Pi backend specification and behavior.

## Acceptance criteria

- The failing test and the full Pi unit/process integration suites pass repeatedly.
- The complete repository validation suite passes.
- No sleeps are added merely to hide the race.
- Changes are committed, integrated into `docs/pi-agent-system-spec`, pushed to PR #6, and the ticket is completed.
