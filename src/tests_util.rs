#![cfg(test)]

use std::time::Duration;

// Helper used by tests: expected drained units for identical parallel branches.
// Assumptions: round-robin routing, identical branch proc times, steady-state.
pub fn expected_drained_identical_parallel(
    endtime: Duration,
    first_exit: Duration,
    source_proc: Duration,
    station_a_proc: Duration,
    branch_proc: Duration,
    num_branches: u128,
) -> usize {
    let branch_effective_ms = branch_proc.as_millis() / num_branches;
    let steady_interval_ms = std::cmp::max(
        source_proc.as_millis(),
        std::cmp::max(station_a_proc.as_millis(), branch_effective_ms),
    );
    if endtime < first_exit {
        0usize
    } else {
        let after_first_ms = endtime.as_millis().saturating_sub(first_exit.as_millis());
        1usize + (after_first_ms / steady_interval_ms) as usize
    }
}
