use super::*;
use std::os::unix::process::ExitStatusExt;
#[test]
fn exit_status_maps_success_and_exact_failure_code() {
    assert_eq!(
        RunOutcome::from_exit_status(ExitStatus::from_raw(0)),
        RunOutcome::Completed
    );
    assert_eq!(
        RunOutcome::from_exit_status(ExitStatus::from_raw(7 << 8)),
        RunOutcome::Failed(ExitCode::from(7))
    );
    assert_eq!(
        RunOutcome::from_exit_status(ExitStatus::from_raw(9)),
        RunOutcome::Failed(ExitCode::from(1))
    );
}
