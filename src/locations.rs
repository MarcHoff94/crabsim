//! Where each MU in a run is: the object holding it, by MU id. Every move writes it, so anything
//! can find an MU by its id alone, user code above all (ADR 0029).
//!
//! MU ids are handed out one after another, so the table is a window over the ids from the oldest
//! MU still in the run to the newest, one object id each: a move is one write, with no hashing. A
//! Drain takes its MUs out, and the window moves on past those gone from its front. An MU that stays
//! while many come and go after it, such as one held in a Buffer for days, would keep the window
//! growing, so past `WINDOW` ids the oldest still there move to a map of their own. Starting MUs,
//! whose ids are the model's, are in the map from Init. Memory stays flat however long a run goes.

use std::collections::HashMap;

/// The most ids the window spans before its oldest MUs move to the map.
const WINDOW: usize = 4096;

/// No MU has this id any more: gone into a Drain, or never made.
const GONE: u32 = u32::MAX;

#[derive(Debug, Clone, Default)]
pub struct MuLocations {
    /// The id of the window's first slot.
    base: u32,
    /// The object holding each MU from `base` on, or `GONE`. A plain `Vec`, so a move is a bounds
    /// check and a write: the slots gone from its front are dropped in batches (`front`).
    window: Vec<u32>,
    /// The first slot that may still hold an MU: those before it are gone.
    front: usize,
    /// MUs with ids before `base` still in the run: starting MUs and those that stayed long.
    long: HashMap<u32, u32>,
}

impl MuLocations {
    /// An empty table whose window starts at `first_id`, the first id a run hands out.
    pub fn new(first_id: u32) -> MuLocations {
        MuLocations { base: first_id, window: Vec::new(), front: 0, long: HashMap::new() }
    }

    /// The object holding the MU, if it is in the run.
    pub fn of(&self, mu: u32) -> Option<u32> {
        match self.slot(mu) {
            Some(at) => self.window.get(at).copied().filter(|&object| object != GONE),
            None => self.long.get(&mu).copied(),
        }
    }

    /// The MU's slot in the window, or `None` for an id before it, kept in the map.
    #[inline]
    fn slot(&self, mu: u32) -> Option<usize> {
        mu.checked_sub(self.base).map(|at| at as usize).filter(|&at| at >= self.front)
    }

    /// The MU is at `object` now: made there, or moved in.
    #[inline]
    pub fn set(&mut self, mu: u32, object: u32) {
        match self.slot(mu) {
            Some(at) => {
                if let Some(slot) = self.window.get_mut(at) {
                    *slot = object;
                } else {
                    self.grow(at, object);
                }
            }
            None => {
                self.long.insert(mu, object);
            }
        }
    }

    /// A new MU beyond the window's end: the window takes it, and moves its oldest MUs to the map
    /// if it spans too many ids.
    #[cold]
    fn grow(&mut self, at: usize, object: u32) {
        self.window.resize(at, GONE);
        self.window.push(object);
        // Too many ids since the oldest MU still there: the oldest move to the map.
        while self.window.len() - self.front > WINDOW {
            let oldest = self.window[self.front];
            if oldest != GONE {
                self.long.insert(self.base + self.front as u32, oldest);
                self.window[self.front] = GONE;
            }
            self.front += 1;
        }
        self.skip_gone();
        self.drop_gone();
    }

    /// Moves `front` past the slots gone.
    #[inline]
    fn skip_gone(&mut self) {
        while self.window.get(self.front) == Some(&GONE) {
            self.front += 1;
        }
    }

    /// Drops the slots gone from the window's front, once there are as many as there are left,
    /// so each slot is moved at most once on average.
    fn drop_gone(&mut self) {
        if self.front >= 1024 && self.front * 2 >= self.window.len() {
            self.window.drain(..self.front);
            self.base += self.front as u32;
            self.front = 0;
        }
    }

    /// The MU has left the run, into a Drain.
    #[inline]
    pub fn gone(&mut self, mu: u32) {
        match self.slot(mu) {
            Some(at) => {
                if let Some(slot) = self.window.get_mut(at) {
                    *slot = GONE;
                }
                if at == self.front {
                    self.skip_gone();
                    self.drop_gone();
                }
            }
            None => {
                self.long.remove(&mu);
            }
        }
    }

    /// How many MUs it knows of, for its tests.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.window.iter().filter(|&&object| object != GONE).count() + self.long.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_mu_is_where_it_moved_to_and_gone_once_it_leaves() {
        let mut locations = MuLocations::new(10);
        locations.set(10, 0);
        locations.set(11, 0);
        locations.set(10, 3);
        assert_eq!((locations.of(10), locations.of(11), locations.of(12)), (Some(3), Some(0), None));
        locations.gone(10);
        assert_eq!(locations.of(10), None);
        assert_eq!(locations.front, 1, "the window moved on past the MU that left");
    }

    #[test]
    fn starting_mus_before_the_first_id_are_kept_apart() {
        let mut locations = MuLocations::new(100);
        locations.set(7, 2);
        assert_eq!(locations.of(7), Some(2));
        locations.gone(7);
        assert_eq!(locations.of(7), None);
    }

    #[test]
    fn an_mu_that_stays_while_many_pass_is_still_found_and_memory_stays_flat() {
        let mut locations = MuLocations::new(1);
        locations.set(1, 5);
        for mu in 2..100_000 {
            locations.set(mu, 1);
            locations.gone(mu);
        }
        assert_eq!(locations.of(1), Some(5), "the one that stayed");
        locations.set(1, 6);
        for mu in 100_000..110_000 {
            locations.set(mu, 1);
            locations.gone(mu);
        }
        assert_eq!(locations.of(1), Some(6), "moved again after it went to the map");
        assert_eq!(locations.len(), 1);
        assert!(locations.window.len() <= 2 * WINDOW, "{}", locations.window.len());
        locations.gone(1);
        assert_eq!(locations.len(), 0);
    }

    #[test]
    fn an_mu_moved_after_it_went_to_the_map_is_found_where_it_moved() {
        let mut locations = MuLocations::new(1);
        locations.set(1, 5);
        // Every MU stays, so the first goes to the map while its slot is still in the window.
        for mu in 2..=(WINDOW as u32 + 1) {
            locations.set(mu, 1);
        }
        locations.set(1, 6);
        for mu in 2..=(WINDOW as u32 + 1) {
            locations.gone(mu);
        }
        assert_eq!(locations.of(1), Some(6));
        locations.gone(1);
        assert_eq!((locations.of(1), locations.len()), (None, 0));
    }
}
