//! Hooks that misbehave, each in its own way, for the sandbox's tests.

use crabsim_guest::*;

hook! {
    /// Never ends.
    fn spin(_call: Call) {
        let mut n: u64 = 0;
        loop {
            n = std::hint::black_box(n.wrapping_add(1));
        }
    }

    /// Takes `megabytes` of memory and keeps it.
    fn hog(_call: Call, megabytes: Int) {
        with_state(|kept: &mut Vec<Vec<u8>>| kept.push(vec![1u8; (megabytes as usize) << 20]));
    }

    /// Panics with its message.
    fn boom(_call: Call, message: Text) {
        panic!("{message}");
    }

    /// Answers a number that is no successor.
    fn nonsense(_call: Call) -> Answer {
        Answer::Successor(Object(9999))
    }

    /// Moves the call's MU to `to`, and logs whether it could.
    fn send(call: Call, to: Object) {
        if let Some(mu) = call.mu() {
            match mu.move_to(to) {
                Ok(()) => log!("moved MU {} to {}", mu.id(), to.name()),
                Err(refusal) => log!("refused: {refusal}"),
            }
        }
    }

    /// Counts its calls and logs the count, to see state kept and cleared.
    fn count(_call: Call) {
        let n = with_state(|n: &mut u32| {
            *n += 1;
            *n
        });
        log!("call {n}");
    }
}
