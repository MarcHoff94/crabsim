//! What the view-state tests share: a real live session on a fake clock, and the view fed from it
//! as the window would feed it.

#![allow(dead_code)]

use crabsim::{demo, Clock, LiveSession};
use crabsim_ui::view::ViewState;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
pub struct FakeClock(Arc<Mutex<Duration>>);

impl FakeClock {
    pub fn set(&self, elapsed: Duration) {
        *self.0.lock().unwrap() = elapsed;
    }
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

pub const SOURCE: u32 = 0;
pub const STATION_1: u32 = 1;
pub const STATION_2: u32 = 2;

/// The demo model in a live session, and a view of it.
pub struct Watched {
    pub session: LiveSession,
    pub clock: FakeClock,
    pub view: ViewState,
}

impl Watched {
    pub fn demo() -> Watched {
        let model = demo::model();
        let view = ViewState::new(&model);
        let clock = FakeClock::default();
        let session = LiveSession::new(move || model.build(0), clock.clone());
        Watched { session, clock, view }
    }

    /// Waits for the session to handle everything sent and reach the clock, then feeds the view
    /// every observation, as one frame of the window would.
    pub fn frame(&mut self) {
        self.session.sync().unwrap();
        let wall = self.clock.elapsed();
        for observation in self.session.observations().try_iter() {
            self.view.observe(&observation, wall);
        }
    }

    pub fn at(&mut self, secs: u64) {
        // Commands sent before take effect at the time on the clock now, not at `secs`.
        self.session.sync().unwrap();
        self.clock.set(Duration::from_secs(secs));
        self.frame();
    }

    /// Asks for a full state as the window does, and takes it in.
    pub fn details(&mut self) {
        self.session.request_state().unwrap();
        self.view.state_requested(self.clock.elapsed());
        self.frame();
    }

    pub fn wall(&self) -> Duration {
        self.clock.elapsed()
    }
}
