//! Example hooks for crabsim: distributions written as code, a timer and a summary at Reset.

use crabsim_guest::*;

hook! {
    /// Sends every n-th MU to `left` and the others to `right`.
    fn every_nth(_call: Call, n: Int, left: Object, right: Object) -> Answer {
        let count = with_state(|count: &mut i64| {
            *count += 1;
            *count
        });
        Answer::Successor(if count % n == 0 { left } else { right })
    }

    /// Sends each MU to the successor holding the fewest MUs, the first of them on a tie.
    fn shortest_queue(call: Call) -> Answer {
        match call.this().successors().into_iter().min_by_key(|successor| successor.mus_held()) {
            Some(successor) => Answer::Successor(successor),
            None => Answer::Default,
        }
    }

    /// Chooses a successor at random, from the deciding object's own stream for code.
    fn random_exit(call: Call) -> Answer {
        let this = call.this();
        let successors = this.successors();
        if successors.is_empty() {
            return Answer::Default;
        }
        let at = (this.random() * successors.len() as f64) as usize;
        Answer::Successor(successors[at.min(successors.len() - 1)])
    }

    /// Answers default, so the object's own strategy chooses: what a hook costs that decides
    /// nothing.
    fn pass(_call: Call) -> Answer {
        Answer::Default
    }

    /// Round robin written as code: each object's successors in turn, waiting for the one whose
    /// turn it is.
    fn cycle(call: Call) -> Answer {
        let this = call.this();
        let count = this.successors().len();
        if count == 0 {
            return Answer::Default;
        }
        let turn = with_state(|turns: &mut std::collections::BTreeMap<u32, usize>| {
            let turn = turns.entry(this.id()).or_insert(0);
            *turn += 1;
            *turn - 1
        });
        Answer::Successor(this.successor(turn % count).expect("within its successors"))
    }

    /// Logs how many MUs `drain` has taken, now and then every `every` after.
    fn report(call: Call, drain: Object, every: Time) -> Result<(), Refusal> {
        log!("{} has taken {} MUs", drain.name(), drain.mus_in());
        if matches!(call.kind, CallKind::AfterInit | CallKind::AfterReset | CallKind::Timer { .. }) {
            call_after(every, 0)?;
        }
        Ok(())
    }
}
