//! A Station with a capacity above 1 (spec issue 110, PR 2, ADR 0001): one machine working on
//! several MUs, with individual or batch start, tested at the run's interface. The models are fed
//! from a Buffer holding starting MUs, so every time below can be worked out by hand.

use crabsim::random_numbers::Distribution;
use crabsim::{
    Capacity, EventType, FailureProfile, ModelDescription, ObjectStatistics, Occurrence, OccurrenceKind, Picture, ProcessState, Recording, SessionOutput,
    SimTime, SimulationRun, StartMode,
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn never_fails() -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0)
}

/// A Station working `process_secs` on each MU, with this capacity and start.
fn station(model: &mut ModelDescription, process_secs: u64, capacity: u64, start: StartMode) -> u32 {
    let id = model.add_station(secs(process_secs), never_fails());
    model.set_station_capacity(id, capacity, start);
    id
}

/// A Buffer holding MUs 1 to `mus`, which offers them at once.
fn stock(model: &mut ModelDescription, mus: u32) -> u32 {
    let buffer = model.add_buffer(Capacity::Unlimited);
    model.set_starting_mus(buffer, &(1..=mus).collect::<Vec<_>>());
    buffer
}

fn connect(model: &mut ModelDescription, connections: &[(u32, u32)]) {
    for &(from, to) in connections {
        model.connect(from, to);
    }
}

fn run_to(model: &ModelDescription, until: u64) -> (SimulationRun, Vec<Occurrence>) {
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    (run, occurrences)
}

/// Each MU that moved from `from` to `to`: its id and when, in seconds.
fn moves(occurrences: &[Occurrence], from: u32, to: u32) -> Vec<(u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { mu_id, from: f, to: t } if f == from && t == to => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect()
}

/// Each MU whose work began at `at`: its id and when, in seconds.
fn starts(occurrences: &[Occurrence], at: u32) -> Vec<(u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { mu_id, at: a, .. } if a == at => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect()
}

fn statistics_of(run: &SimulationRun, id: u32) -> ObjectStatistics {
    run.state().objects[id as usize].statistics
}

fn assert_times_add_up(run: &SimulationRun) {
    let state = run.state();
    for object in &state.objects {
        assert_eq!(object.statistics.measured_time(), state.time, "object {}", object.id);
    }
}

/// Stock of 5, a Station of capacity 3 working 10 s on each, individually, and a Drain.
fn individual_line() -> (ModelDescription, u32, u32) {
    let mut model = ModelDescription::new();
    let buffer = stock(&mut model, 5);
    let station = station(&mut model, 10, 3, StartMode::Individual);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, station), (station, drain)]);
    (model, station, drain)
}

#[test]
fn with_individual_start_each_mu_starts_on_entry_up_to_the_capacity() {
    let (model, station, drain) = individual_line();
    let (run, occurrences) = run_to(&model, 30);
    assert_eq!(starts(&occurrences, station), [(1, 0), (2, 0), (3, 0), (4, 10), (5, 10)]);
    // Finished at once, they leave in the order they came in.
    assert_eq!(moves(&occurrences, station, drain), [(1, 10), (2, 10), (3, 10), (4, 20), (5, 20)]);
    let statistics = statistics_of(&run, station);
    assert_eq!(statistics.time_busy, secs(20));
    assert_eq!(statistics.time_busy_full(), secs(10), "three in work");
    assert_eq!(statistics.time_busy_not_full, secs(10), "two in work, a place free");
    assert_eq!(statistics.time_busy_partly_blocked, SimTime::ZERO);
    assert_eq!(statistics.time_idle, secs(10));
    assert_eq!(statistics.max_contents, 3);
    assert_times_add_up(&run);
}

#[test]
fn a_batch_starts_when_full_and_a_part_batch_waits_idle() {
    let mut model = ModelDescription::new();
    let buffer = stock(&mut model, 5);
    let station = station(&mut model, 10, 2, StartMode::Batch);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, station), (station, drain)]);
    let (run, occurrences) = run_to(&model, 40);
    assert_eq!(starts(&occurrences, station), [(1, 0), (2, 0), (3, 10), (4, 10)]);
    assert_eq!(moves(&occurrences, station, drain), [(1, 10), (2, 10), (3, 20), (4, 20)]);
    let state = run.state();
    let object = &state.objects[station as usize];
    assert_eq!(object.contents, [5], "MU 5 waits for a second");
    assert_eq!(object.process, ProcessState::Idle, "a batch waiting to fill is idle");
    assert_eq!(object.remaining, None);
    let statistics = object.statistics;
    assert_eq!(statistics.time_busy_full(), secs(20));
    assert_eq!(statistics.time_idle, secs(20));
    assert_times_add_up(&run);
}

/// Stock of 4, a Station of capacity 2 working 10 s, then a Station working 15 s on one MU.
fn line_into_a_slower_station(start: StartMode) -> (ModelDescription, u32, u32, u32) {
    let mut model = ModelDescription::new();
    let buffer = stock(&mut model, 4);
    let station = station(&mut model, 10, 2, start);
    let next = model.add_station(secs(15), never_fails());
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, station), (station, next), (next, drain)]);
    (model, buffer, station, next)
}

#[test]
fn a_batch_keeps_its_entrance_closed_until_it_is_empty() {
    let (model, buffer, station, next) = line_into_a_slower_station(StartMode::Batch);
    let (run, occurrences) = run_to(&model, 60);
    // MU 2 can't leave until 25, and the place MU 1 left stays closed until then.
    assert_eq!(moves(&occurrences, buffer, station), [(1, 0), (2, 0), (3, 25), (4, 25)]);
    assert_eq!(moves(&occurrences, station, next), [(1, 10), (2, 25), (3, 40), (4, 55)]);
    let statistics = statistics_of(&run, station);
    // Busy 0-10 and 25-35, blocked with nothing in work 10-25 and 35-55, idle after.
    assert_eq!(statistics.time_busy_full(), secs(20));
    assert_eq!(statistics.time_busy_partly_blocked, SimTime::ZERO);
    assert_eq!(statistics.time_blocked, secs(35));
    assert_eq!(statistics.time_idle, secs(5));
    assert_times_add_up(&run);
}

#[test]
fn with_individual_start_a_free_place_takes_an_mu_while_a_finished_one_waits() {
    let (model, buffer, station, next) = line_into_a_slower_station(StartMode::Individual);
    let (run, occurrences) = run_to(&model, 60);
    assert_eq!(moves(&occurrences, buffer, station), [(1, 0), (2, 0), (3, 10), (4, 25)]);
    // Earliest finished first: MU 2, finished at 10, before MU 3, finished at 20.
    assert_eq!(moves(&occurrences, station, next), [(1, 10), (2, 25), (3, 40), (4, 55)]);
    let statistics = statistics_of(&run, station);
    assert_eq!(statistics.time_busy_full(), secs(10), "0-10");
    assert_eq!(statistics.time_busy_not_full, SimTime::ZERO);
    assert_eq!(statistics.time_busy_partly_blocked, secs(20), "10-20 and 25-35, working while MU 2 or 3 can't leave");
    assert_eq!(statistics.time_blocked, secs(25), "20-25 and 35-55, nothing in work");
    assert_eq!(statistics.time_idle, secs(5));
    assert_times_add_up(&run);
}

#[test]
fn it_reports_blocked_only_while_nothing_is_in_work() {
    let (model, _, station, _) = line_into_a_slower_station(StartMode::Individual);
    let (_, occurrences) = run_to(&model, 60);
    let blocking: Vec<(bool, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::Blocked { object_id } if object_id == station => Some((true, o.time.as_secs())),
            OccurrenceKind::Unblocked { object_id } if object_id == station => Some((false, o.time.as_secs())),
            _ => None,
        })
        .collect();
    // At 10 it is blocked until the stock's retry, a later event at the same instant, brings MU
    // 3 into work. At 25 MU 2 leaves and MU 3 is refused, which is no change at all.
    assert_eq!(blocking, [(true, 10), (false, 10), (true, 20), (false, 25), (true, 35), (false, 55)]);
}

#[test]
fn starting_mus_begin_at_init_unless_a_batch_is_short() {
    for (capacity, start, starting, begun) in
        [(3, StartMode::Individual, 2, 2), (3, StartMode::Batch, 2, 0), (2, StartMode::Batch, 2, 2)]
    {
        let mut model = ModelDescription::new();
        let station = station(&mut model, 10, capacity, start);
        model.set_starting_mus(station, &(1..=starting).collect::<Vec<_>>());
        let drain = model.add_drain();
        model.connect(station, drain);
        let mut run = model.build(0);
        run.set_recording(Recording::ALL);
        let occurrences = run.init().unwrap();
        assert_eq!(starts(&occurrences, station).len(), begun, "capacity {capacity}, {start:?}, {starting} starting");
        let state = run.state();
        assert_eq!(state.objects[station as usize].contents.len(), starting as usize);
        assert_eq!(state.objects[station as usize].statistics.mus_in, u64::from(starting));
    }
}

#[test]
fn a_stop_stops_every_mu_in_work_at_once() {
    let mut model = ModelDescription::new();
    let station = station(&mut model, 10, 2, StartMode::Individual);
    model.set_starting_mus(station, &[1, 2]);
    let drain = model.add_drain();
    model.connect(station, drain);
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    run.init().unwrap();
    let mut occurrences = run.advance_to(secs(4)).unwrap();
    run.schedule_event(secs(4), EventType::PauseOn, station).unwrap();
    occurrences.extend(run.advance_to(secs(5)).unwrap());
    let paused = &run.state().objects[station as usize];
    assert_eq!(paused.remaining, Some(secs(6)), "frozen");
    assert_eq!(paused.work.iter().map(|work| (work.mu_id, work.left)).collect::<Vec<_>>(), [(1, secs(6)), (2, secs(6))]);
    run.schedule_event(secs(10), EventType::PauseOff, station).unwrap();
    occurrences.extend(run.advance_to(secs(30)).unwrap());
    assert_eq!(moves(&occurrences, station, drain), [(1, 16), (2, 16)]);
    let statistics = statistics_of(&run, station);
    assert_eq!(statistics.time_paused, secs(6));
    assert_eq!(statistics.time_busy_full(), secs(10));
    assert_eq!(statistics.stops, 1);
    assert_times_add_up(&run);
}

#[test]
fn reset_returns_it_to_its_default_state() {
    for start in [StartMode::Individual, StartMode::Batch] {
        let (model, _, station, _) = line_into_a_slower_station(start);
        let (mut first, first_occurrences) = run_to(&model, 30);
        let midway = first.state().objects[station as usize].clone();
        assert!(!midway.contents.is_empty());
        first.reset();
        assert_eq!(first.state(), model.build(0).state(), "{start:?}");
        first.set_recording(Recording::ALL);
        let mut again = first.init().unwrap();
        again.extend(first.advance_to(secs(30)).unwrap());
        assert_eq!(again, first_occurrences, "{start:?}");
    }
}

#[test]
fn a_copy_taken_part_way_carries_on_exactly_as_the_original() {
    // A checkpoint is a copy of the run, and going back replays from one.
    let (model, _, _, _) = line_into_a_slower_station(StartMode::Individual);
    let (mut run, _) = run_to(&model, 22);
    let mut copy = run.clone();
    let original = run.advance_to(secs(60)).unwrap();
    let replayed = copy.advance_to(secs(60)).unwrap();
    assert_eq!(replayed, original);
    assert_eq!(copy.state(), run.state());
}

#[test]
fn the_picture_follows_a_station_with_capacity_from_occurrences_alone() {
    for start in [StartMode::Individual, StartMode::Batch] {
        let (mut model, _, station, _) = line_into_a_slower_station(start);
        model.set_station_capacity(station, 3, start);
        let mut run = model.build(0);
        run.set_recording(Recording::ALL);
        run.init().unwrap();
        let mut picture = Picture::new();
        picture.observe(&SessionOutput::State(run.state()));
        let pause = [(secs(12), EventType::PauseOn), (secs(17), EventType::PauseOff)];
        for (at, event) in pause {
            run.schedule_event(at, event, station).unwrap();
        }
        // Until the queue is empty: a part batch waits for ever with nothing scheduled.
        loop {
            let processed = run.events_processed();
            let occurrences = run.step().unwrap();
            if run.events_processed() == processed {
                break;
            }
            if occurrences.is_empty() {
                continue;
            }
            picture.observe(&SessionOutput::Occurrences(occurrences));
            let mut from_state = Picture::new();
            from_state.observe(&SessionOutput::State(run.state()));
            assert_eq!(picture.objects(), from_state.objects(), "{start:?} at {}", run.state().time);
        }
    }
}
