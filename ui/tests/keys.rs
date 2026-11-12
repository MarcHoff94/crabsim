//! The window's keys (spec issue 99): what each typed key asks for, counts, two-key sequences,
//! the prompts, and the helpers they need. No window.

use crabsim_ui::keys::{address, attribute_matches, matches, nearest, Address, Command, Direction, Key, Keys, PromptKind};
use crabsim_ui::view::Point;

fn typed(keys: &mut Keys, text: &str) -> Vec<Command> {
    text.chars().flat_map(|c| keys.press(Key::Char(c))).collect()
}

#[test]
fn each_key_asks_for_its_command() {
    let mut keys = Keys::default();
    let expected = [
        ('h', Command::Move(Direction::Left, 1)),
        ('j', Command::Move(Direction::Down, 1)),
        ('k', Command::Move(Direction::Up, 1)),
        ('l', Command::Move(Direction::Right, 1)),
        ('>', Command::Successor(1)),
        ('<', Command::Predecessor(1)),
        ('G', Command::Last),
        ('d', Command::Delete),
        ('0', Command::Fit),
        (' ', Command::StartOrHalt),
        ('x', Command::StepForward(1)),
        ('y', Command::StepBack(1)),
        ('R', Command::Reset),
        ('+', Command::Faster),
        ('-', Command::Slower),
        ('a', Command::Animate),
        ('n', Command::NextMatch { backwards: false }),
        ('N', Command::NextMatch { backwards: true }),
        ('?', Command::Help),
    ];
    for (key, command) in expected {
        assert_eq!(keys.press(Key::Char(key)), vec![command], "{key}");
    }
    assert_eq!(keys.press(Key::Char('q')), Vec::<Command>::new(), "a key that means nothing");
    assert_eq!(keys.press(Key::Enter), vec![Command::Edit]);
    assert_eq!(keys.press(Key::Escape), vec![Command::Unselect]);
    assert_eq!(keys.press(Key::Up), vec![Command::Pan(Direction::Up)]);
    assert_eq!(keys.press(Key::ZoomIn), vec![Command::Zoom { closer: true }]);
    assert_eq!(keys.press(Key::ShiftTab), vec![Command::Step { backwards: true }]);
}

#[test]
fn two_key_sequences_wait_for_their_second_key() {
    let mut keys = Keys::default();
    assert_eq!(typed(&mut keys, "gg"), vec![Command::First]);
    assert_eq!(typed(&mut keys, "gt"), vec![Command::GoTo]);
    assert_eq!(typed(&mut keys, "zz"), vec![Command::Centre]);
    assert_eq!(typed(&mut keys, "t3"), vec![Command::Tab(3)], "a digit alone would begin a count");
    assert_eq!(typed(&mut keys, "gl"), vec![Command::Move(Direction::Right, 1)], "not a sequence: the second key counts alone");
}

#[test]
fn a_count_repeats_motions_and_steps_and_picks_an_exit_on_the_flow_keys() {
    let mut keys = Keys::default();
    assert_eq!(typed(&mut keys, "5l"), vec![Command::Move(Direction::Right, 5)]);
    assert_eq!(typed(&mut keys, "10x"), vec![Command::StepForward(10)]);
    assert_eq!(typed(&mut keys, "12y"), vec![Command::StepBack(12)]);
    assert_eq!(typed(&mut keys, "3>2>1>"), vec![Command::Successor(3), Command::Successor(2), Command::Successor(1)]);
    assert_eq!(typed(&mut keys, "2<"), vec![Command::Predecessor(2)]);
    assert_eq!(typed(&mut keys, "20"), Vec::<Command>::new(), "a count of 20, waiting");
    assert_eq!(keys.count(), Some(20));
    assert_eq!(typed(&mut keys, "j"), vec![Command::Move(Direction::Down, 20)]);
    assert_eq!(typed(&mut keys, "5R"), vec![Command::Reset], "other keys ignore a count");
    assert_eq!(typed(&mut keys, "2gg"), vec![Command::First]);
}

#[test]
fn visual_selection_starts_and_ends_with_v_or_escape() {
    let mut keys = Keys::default();
    assert_eq!(typed(&mut keys, "v"), vec![Command::Visual(true)]);
    assert!(keys.visual());
    assert_eq!(keys.press(Key::Escape), vec![Command::Visual(false)], "Esc ends it, keeping the selection");
    assert_eq!(keys.press(Key::Escape), vec![Command::Unselect], "then unselects");
    typed(&mut keys, "v");
    assert_eq!(typed(&mut keys, "v"), vec![Command::Visual(false)]);
}

#[test]
fn a_prompt_takes_every_key_until_enter_or_escape() {
    let mut keys = Keys::default();
    assert_eq!(typed(&mut keys, "/"), vec![Command::Prompted(PromptKind::Search, String::new())]);
    let changes = typed(&mut keys, "Mx");
    assert_eq!(changes.last(), Some(&Command::Prompted(PromptKind::Search, "Mx".into())), "x types, it doesn't step");
    assert_eq!(keys.press(Key::Backspace), vec![Command::Prompted(PromptKind::Search, "M".into())]);
    assert_eq!(keys.press(Key::Tab), vec![Command::Complete(PromptKind::Search, "M".into())]);
    assert_eq!(keys.press(Key::Enter), vec![Command::Answered(PromptKind::Search, "M".into())]);
    assert_eq!(keys.prompt(), None);

    typed(&mut keys, "e");
    assert_eq!(keys.prompt(), Some((PromptKind::Edit, "")));
    typed(&mut keys, "pro");
    keys.set_prompt_text("process_time");
    assert_eq!(keys.prompt(), Some((PromptKind::Edit, "process_time")), "Tab completes it");
    assert_eq!(keys.press(Key::Escape), vec![Command::PromptClosed(PromptKind::Edit)]);
    assert_eq!(typed(&mut keys, "o"), vec![Command::Prompted(PromptKind::Open, String::new())]);
}

fn at(x: f64, y: f64) -> Point {
    Point { x, y }
}

#[test]
fn the_nearest_object_in_a_direction_is_ahead_and_within_60_degrees() {
    // A cross around the middle, and one far off up and to the right.
    let centres = [at(0.0, 0.0), at(5.0, 0.0), at(-4.0, 0.5), at(0.0, 3.0), at(0.3, -6.0), at(6.0, 6.0)];
    assert_eq!(nearest(&centres, 0, Direction::Right), Some(1));
    assert_eq!(nearest(&centres, 0, Direction::Left), Some(2));
    assert_eq!(nearest(&centres, 0, Direction::Up), Some(3));
    assert_eq!(nearest(&centres, 0, Direction::Down), Some(4));
    assert_eq!(nearest(&centres, 1, Direction::Right), None, "nothing further right");
    assert_eq!(nearest(&centres, 3, Direction::Right), Some(1), "down and to the right, nearer than up and to the right");
    assert_eq!(nearest(&[at(0.0, 0.0), at(1.0, 5.0)], 0, Direction::Right), None, "more than 60° off");
}

#[test]
fn names_match_exactly_first_then_by_part_ignoring_case() {
    let names = ["Station 1", "Station 10", "Mill", "station 1"];
    assert_eq!(matches(&names, "station 1"), vec![0, 1, 3], "the first exact one ignoring case first, then those containing it");
    assert_eq!(matches(&names, "Mill"), vec![2]);
    assert_eq!(matches(&names, "MI"), vec![2]);
    assert_eq!(matches(&names, "  "), Vec::<usize>::new());
}

#[test]
fn an_address_is_an_attribute_or_an_object_and_an_attribute() {
    assert_eq!(address("proc"), Address { object: None, attribute: "proc" });
    assert_eq!(address("Station 3.proc"), Address { object: Some("Station 3"), attribute: "proc" });
    assert_eq!(address("Line 1.2.proc"), Address { object: Some("Line 1.2"), attribute: "proc" }, "split at the last dot");
    assert!(attribute_matches("proc", "process_time", "Process time"));
    assert!(attribute_matches("ProcessTime", "process_time", "Process time"));
    assert!(attribute_matches("name", "name", "Name"));
    assert!(!attribute_matches("fail", "process_time", "Process time"));
    assert!(!attribute_matches("", "process_time", "Process time"));
}
