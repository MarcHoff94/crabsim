//! Where each MU in a run is: the object holding it, by MU id. Every move writes it, so anything
//! can find an MU by its id alone, user code above all (ADR 0029).
//!
//! MU ids are handed out one after another, so the table is a window over the ids from the oldest
//! MU still in the run to the newest, one object id each: a move is one write, with no hashing. A
//! Drain takes its MUs out, and the window moves on past those gone from its front. While at least
//! half its slots hold an MU, the window grows with the MUs in the run, at most 8 bytes each, so a
//! Buffer holding 50,000 stays in it. An MU that stays while many come and go after it, such as one
//! held in a Buffer for days, would leave the window mostly empty, so once it spans more than
//! `WINDOW` ids and less than half of them hold an MU, the oldest still there move to a map of their
//! own. Starting MUs, whose ids are the model's, are in the map from Init. Memory stays flat however
//! long a run goes. The `crowded-buffer` benchmark scenario measures the crowded case: with the map
//! taking every MU past 4,096 ids, it ran a third slower (`docs/design/performance-trade-offs.md`).

use std::collections::HashMap;

/// The most ids the window spans before its oldest MUs move to the map, if less than half of them
/// hold an MU.
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
    /// About how many slots from `front` on hold an MU, which decides only when MUs move to the map:
    /// counted as MUs are made, leave and move there, never as they move.
    live: usize,
    /// MUs with ids before `base` still in the run: starting MUs and those that stayed long.
    long: HashMap<u32, u32>,
}

impl MuLocations {
    /// An empty table whose window starts at `first_id`, the first id a run hands out.
    pub fn new(first_id: u32) -> MuLocations {
        MuLocations { base: first_id, window: Vec::new(), front: 0, live: 0, long: HashMap::new() }
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
                // A slot empties only as its MU leaves the run or moves to the map, so a move into
                // the window always finds its MU's slot full.
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
    /// if it spans too many ids, most of them gone.
    #[cold]
    fn grow(&mut self, at: usize, object: u32) {
        self.window.resize(at, GONE);
        self.window.push(object);
        self.live += 1;
        while self.window.len() - self.front > WINDOW && self.live * 2 < self.window.len() - self.front {
            let oldest = self.window[self.front];
            if oldest != GONE {
                self.long.insert(self.base + self.front as u32, oldest);
                self.window[self.front] = GONE;
                self.live = self.live.saturating_sub(1);
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
                    if *slot != GONE {
                        self.live = self.live.saturating_sub(1);
                    }
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
    fn a_window_mostly_holding_mus_grows_with_them_rather_than_moving_them_to_the_map() {
        let mut locations = MuLocations::new(1);
        // 50,000 MUs in the run at once, as in a crowded Buffer, then each leaving in turn as
        // another comes.
        for mu in 1..=50_000 {
            locations.set(mu, 2);
        }
        for mu in 1..=100_000u32 {
            locations.set(mu + 50_000, 2);
            locations.set(mu, 3);
            assert_eq!(locations.of(mu), Some(3));
            locations.gone(mu);
        }
        assert!(locations.long.is_empty(), "{} in the map", locations.long.len());
        assert_eq!(locations.len(), 50_000);
        assert!(locations.window.len() <= 2 * 50_000 + 1024, "at most 8 bytes for each MU in it: {}", locations.window.len());
        assert_eq!((locations.of(100_001), locations.of(150_000), locations.of(100_000)), (Some(2), Some(2), None));
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
