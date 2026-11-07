//! MUs drawn in the objects that hold them, and counts where they can't be followed, tested
//! without a window.

mod common;

use common::{Watched, SOURCE, STATION_1};
use crabsim::{demo, SessionStatus, SimTime, Observation};
use crabsim_ui::camera::Camera;
use crabsim_ui::view::{Point, ViewState, MU_MAX_SPEED, MU_METRES, MU_MIN_PIXELS_PER_METRE};
use std::time::Duration;

fn camera(pixels_per_metre: f64) -> Camera {
    Camera { centre: Point { x: 0.0, y: 0.0 }, pixels_per_metre }
}

/// Close enough to follow MUs.
fn close() -> Camera {
    camera(MU_MIN_PIXELS_PER_METRE * 2.0)
}

/// The demo run to `secs` at speed 1, then halted.
fn halted_at(secs: u64) -> Watched {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(secs);
    watched.session.halt().unwrap();
    watched.frame();
    watched
}

#[test]
fn each_mu_is_drawn_inside_the_object_that_holds_it() {
    // At 12 s Station 1 works on the first MU and the Source holds the second, waiting.
    let watched = halted_at(12);
    let view = &watched.view;
    let markers = view.mu_markers(&close());
    let held: Vec<u32> = [SOURCE, STATION_1].iter().flat_map(|&id| view.picture().object(id).unwrap().contents().to_vec()).collect();
    let mut drawn: Vec<u32> = markers.iter().map(|marker| marker.mu).collect();
    drawn.sort();
    let mut expected = held.clone();
    expected.sort();
    assert_eq!(drawn, expected, "every MU held, once");
    for marker in &markers {
        assert_eq!(view.object_at(marker.at), Some(marker.holder), "MU {} inside its holder", marker.mu);
        assert!(view.picture().object(marker.holder).unwrap().contents().contains(&marker.mu));
    }
}

#[test]
fn a_moved_mu_is_drawn_in_its_new_holder_at_once() {
    let mut watched = Watched::demo();
    watched.session.step_forward().unwrap();
    watched.frame();
    // From Init the Source holds the MU it is working on.
    let markers = watched.view.mu_markers(&close());
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].holder, SOURCE);
    let first = markers[0].mu;
    // At 5 s that MU moves straight on to Station 1, and the Source starts on the next.
    watched.session.step_forward().unwrap();
    watched.frame();
    let markers = watched.view.mu_markers(&close());
    let moved = markers.iter().find(|marker| marker.mu == first).unwrap();
    assert_eq!(moved.holder, STATION_1);
    assert_eq!(moved.at, watched.view.objects()[STATION_1 as usize].centre, "one MU sits in the middle");
    assert_eq!(markers.len(), 2);
}

#[test]
fn going_back_puts_mus_where_they_were_then() {
    let mut watched = halted_at(30);
    watched.session.go_back(SimTime::from_secs(5)).unwrap();
    watched.frame();
    let holders: Vec<u32> = watched.view.mu_markers(&close()).iter().map(|marker| marker.holder).collect();
    assert_eq!(holders, vec![SOURCE, STATION_1]);
}

#[test]
fn mus_show_only_when_zoomed_in_far_enough() {
    let watched = halted_at(12);
    assert!(!watched.view.mu_markers(&close()).is_empty());
    assert!(watched.view.mu_markers(&camera(MU_MIN_PIXELS_PER_METRE * 0.9)).is_empty());
    assert!(MU_METRES * MU_MIN_PIXELS_PER_METRE >= 3.0, "an MU is at least 3 pixels across when shown");
}

#[test]
fn mus_show_while_advancing_only_slowly_enough_to_follow() {
    let mut view = ViewState::new(&demo::model());
    let wall = Duration::ZERO;
    let mut run = demo::model().build(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(12)).unwrap();
    view.observe(&Observation::State(run.state()), wall);
    assert!(view.shows_mus(&close()), "halted, whatever the speed");
    view.set_speed(MU_MAX_SPEED * 10.0);
    assert!(view.shows_mus(&close()), "halted, nothing moves");
    view.observe(&Observation::Status(SessionStatus::Advancing), wall);
    assert!(!view.shows_mus(&close()), "too fast to follow");
    view.set_speed(MU_MAX_SPEED);
    assert!(view.shows_mus(&close()));
}

#[test]
fn several_mus_spread_out_inside_their_holder() {
    let view = ViewState::new(&demo::model());
    let station = &view.objects()[STATION_1 as usize];
    for count in 1..=9 {
        let spots = station.mu_spots(count);
        assert_eq!(spots.len(), count);
        for (index, spot) in spots.iter().enumerate() {
            assert_eq!(view.object_at(*spot), Some(STATION_1), "{count} MUs: spot {index} inside");
            for other in &spots[..index] {
                assert!((spot.x - other.x).hypot(spot.y - other.y) > 1e-6, "{count} MUs: spots apart");
            }
        }
    }
}

#[test]
fn where_mus_are_too_fast_to_follow_holders_show_a_count() {
    let mut watched = halted_at(12);
    assert!(watched.view.counts(&close()).is_empty(), "MUs are drawn instead");
    watched.view.set_speed(MU_MAX_SPEED * 10.0);
    watched.session.start().unwrap();
    watched.frame();
    assert_eq!(watched.view.status(), SessionStatus::Advancing);
    assert!(watched.view.mu_markers(&close()).is_empty());
    let mut counts = watched.view.counts(&close());
    counts.sort();
    assert_eq!(counts, vec![(SOURCE, 1), (STATION_1, 1)]);
    assert!(watched.view.counts(&camera(1.0)).is_empty(), "too small to read");
}
