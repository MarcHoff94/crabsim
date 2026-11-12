//! The window's keyboard (spec issue 99): vim-like keys, read as the characters typed, turned
//! into commands. It holds counts, two-key sequences such as `gg`, the visual selection and the
//! one-line prompts, and has the pure helpers the commands need: the nearest object in a
//! direction, matching names, and `object.attribute` addresses. Plain Rust with no egui types, so
//! it is tested without a window.

use crate::view::Point;

/// A key as the window reads it: a character typed, or one of the keys that types none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    ShiftTab,
    Up,
    Down,
    Left,
    Right,
    /// Ctrl and `+`.
    ZoomIn,
    /// Ctrl and `-`.
    ZoomOut,
}

/// A direction on the canvas: up is north, as the floor is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

/// Which one-line prompt is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// `/`: find an object by part of its name.
    Search,
    /// `e`: go to a value, of the selected object or of `object.attribute`.
    Edit,
    /// `o`: open a model file by part of its path.
    Open,
}

impl PromptKind {
    /// What the prompt starts with on screen.
    pub fn sign(self) -> &'static str {
        match self {
            PromptKind::Search => "/",
            PromptKind::Edit => "e ",
            PromptKind::Open => "open ",
        }
    }
}

/// What a key asks the window to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// To the nearest object in this direction, `count` times.
    Move(Direction, u32),
    /// To the successor at this exit, from 1.
    Successor(usize),
    /// To the predecessor at this entrance, from 1.
    Predecessor(usize),
    First,
    Last,
    /// Tab and Shift+Tab: the next or previous object in model order.
    Step { backwards: bool },
    Unselect,
    /// Visual selection began or ended: motions add the objects they land on.
    Visual(bool),
    Delete,
    Pan(Direction),
    Zoom { closer: bool },
    Fit,
    Centre,
    StartOrHalt,
    StepForward(u32),
    StepBack(u32),
    Reset,
    Faster,
    Slower,
    Animate,
    /// Into the Go to field.
    GoTo,
    /// To the tab at this place, from 1: Inspector, Results, Paths, Agents.
    Tab(usize),
    /// Into the selected object's first field.
    Edit,
    /// A prompt's text changed, or it opened.
    Prompted(PromptKind, String),
    /// Tab in a prompt: complete, or go to the next candidate.
    Complete(PromptKind, String),
    /// Enter in a prompt.
    Answered(PromptKind, String),
    /// Esc in a prompt.
    PromptClosed(PromptKind),
    /// `n` and `N`: the next or previous match of the last search.
    NextMatch { backwards: bool },
    Help,
}

/// What has been typed so far that isn't a command yet.
#[derive(Debug, Default, Clone)]
pub struct Keys {
    count: Option<u32>,
    /// The first key of a two-key sequence: `g`, `z` or `t`.
    pending: Option<char>,
    prompt: Option<(PromptKind, String)>,
    visual: bool,
}

impl Keys {
    /// The prompt open and its text, if any.
    pub fn prompt(&self) -> Option<(PromptKind, &str)> {
        self.prompt.as_ref().map(|(kind, text)| (*kind, text.as_str()))
    }

    /// Replaces the open prompt's text, as completing does.
    pub fn set_prompt_text(&mut self, text: impl Into<String>) {
        if let Some((_, typed)) = self.prompt.as_mut() {
            *typed = text.into();
        }
    }

    /// Whether motions add to the selection.
    pub fn visual(&self) -> bool {
        self.visual
    }

    /// The count typed so far, for showing it.
    pub fn count(&self) -> Option<u32> {
        self.count
    }

    /// Takes one key, giving what it asks for: nothing while a count or sequence is still being
    /// typed, or a key that means nothing.
    pub fn press(&mut self, key: Key) -> Vec<Command> {
        if let Some((kind, text)) = self.prompt.as_mut() {
            let kind = *kind;
            return match key {
                Key::Char(c) => {
                    text.push(c);
                    vec![Command::Prompted(kind, text.clone())]
                }
                Key::Backspace => {
                    text.pop();
                    vec![Command::Prompted(kind, text.clone())]
                }
                Key::Tab => vec![Command::Complete(kind, text.clone())],
                Key::Enter => {
                    let text = std::mem::take(text);
                    self.prompt = None;
                    vec![Command::Answered(kind, text)]
                }
                Key::Escape => {
                    self.prompt = None;
                    vec![Command::PromptClosed(kind)]
                }
                _ => Vec::new(),
            };
        }
        // The second key of a sequence.
        if let Some(first) = self.pending.take() {
            let count = self.count.take();
            return match (first, key) {
                ('g', Key::Char('g')) => vec![Command::First],
                ('g', Key::Char('t')) => vec![Command::GoTo],
                ('z', Key::Char('z')) => vec![Command::Centre],
                // A digit alone begins a count, so a tab is `t` and its place.
                ('t', Key::Char(tab @ '1'..='4')) => vec![Command::Tab(tab.to_digit(10).expect("a digit") as usize)],
                _ => {
                    // Not a sequence: the key counts on its own.
                    self.count = count;
                    self.press(key)
                }
            };
        }
        if let Key::Char(digit @ '0'..='9') = key {
            // 0 begins no count: it fits the model.
            if digit != '0' || self.count.is_some() {
                let value = digit.to_digit(10).expect("a digit");
                self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(value).min(100_000));
                return Vec::new();
            }
        }
        let count = self.count.take();
        let times = count.unwrap_or(1).max(1);
        let place = count.unwrap_or(1).max(1) as usize;
        let open = |keys: &mut Keys, kind: PromptKind| {
            keys.prompt = Some((kind, String::new()));
            vec![Command::Prompted(kind, String::new())]
        };
        match key {
            Key::Char('h') => vec![Command::Move(Direction::Left, times)],
            Key::Char('j') => vec![Command::Move(Direction::Down, times)],
            Key::Char('k') => vec![Command::Move(Direction::Up, times)],
            Key::Char('l') => vec![Command::Move(Direction::Right, times)],
            Key::Char('>') => vec![Command::Successor(place)],
            Key::Char('<') => vec![Command::Predecessor(place)],
            Key::Char('G') => vec![Command::Last],
            Key::Char(first @ ('g' | 'z' | 't')) => {
                self.pending = Some(first);
                self.count = count;
                Vec::new()
            }
            Key::Char('v') => {
                self.visual = !self.visual;
                vec![Command::Visual(self.visual)]
            }
            Key::Char('d') => vec![Command::Delete],
            Key::Char('0') => vec![Command::Fit],
            Key::Char(' ') => vec![Command::StartOrHalt],
            Key::Char('x') => vec![Command::StepForward(times)],
            Key::Char('y') => vec![Command::StepBack(times)],
            Key::Char('R') => vec![Command::Reset],
            Key::Char('+') => vec![Command::Faster],
            Key::Char('-') => vec![Command::Slower],
            Key::Char('a') => vec![Command::Animate],
            Key::Char('/') => open(self, PromptKind::Search),
            Key::Char('e') => open(self, PromptKind::Edit),
            Key::Char('o') => open(self, PromptKind::Open),
            Key::Char('n') => vec![Command::NextMatch { backwards: false }],
            Key::Char('N') => vec![Command::NextMatch { backwards: true }],
            Key::Char('?') => vec![Command::Help],
            Key::Enter => vec![Command::Edit],
            Key::Escape if self.visual => {
                self.visual = false;
                vec![Command::Visual(false)]
            }
            Key::Escape => vec![Command::Unselect],
            Key::Tab => vec![Command::Step { backwards: false }],
            Key::ShiftTab => vec![Command::Step { backwards: true }],
            Key::Up => vec![Command::Pan(Direction::Up)],
            Key::Down => vec![Command::Pan(Direction::Down)],
            Key::Left => vec![Command::Pan(Direction::Left)],
            Key::Right => vec![Command::Pan(Direction::Right)],
            Key::ZoomIn => vec![Command::Zoom { closer: true }],
            Key::ZoomOut => vec![Command::Zoom { closer: false }],
            _ => Vec::new(),
        }
    }

    /// Ends the visual selection, as when the selection is cleared some other way.
    pub fn end_visual(&mut self) {
        self.visual = false;
    }
}

/// The nearest object from `from` in `direction`, by centre: one within 60° either side of it,
/// the one ahead counting more than the one off to the side. `None` when there is none.
pub fn nearest(centres: &[Point], from: usize, direction: Direction) -> Option<usize> {
    let origin = centres.get(from)?;
    let (ax, ay) = match direction {
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        Direction::Up => (0.0, 1.0),
        Direction::Down => (0.0, -1.0),
    };
    centres
        .iter()
        .enumerate()
        .filter(|&(index, _)| index != from)
        .filter_map(|(index, centre)| {
            let (dx, dy) = (centre.x - origin.x, centre.y - origin.y);
            let ahead = dx * ax + dy * ay;
            let aside = (dx * ay - dy * ax).abs();
            // Ahead, and within 60° of straight ahead.
            (ahead > 1e-9 && aside <= ahead * 3f64.sqrt()).then_some((index, ahead + 2.0 * aside))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(index, _)| index)
}

/// The names matching `text`, by place: an exact name first, then those containing it, ignoring
/// case, each in order. Nothing for empty text.
pub fn matches(names: &[&str], text: &str) -> Vec<usize> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Vec::new();
    }
    let exact = names.iter().position(|name| name.to_lowercase() == text);
    let mut found: Vec<usize> = exact.into_iter().collect();
    found.extend((0..names.len()).filter(|&index| Some(index) != exact && names[index].to_lowercase().contains(&text)));
    found
}

/// Whether an attribute is meant by `typed`: part of its name in a file or of its label, ignoring
/// case, spaces and underscores, such as `proc` or `Process time` for `process_time`.
pub fn attribute_matches(typed: &str, name: &str, label: &str) -> bool {
    let plain = |text: &str| text.to_lowercase().replace([' ', '_'], "");
    let typed = plain(typed);
    !typed.is_empty() && (plain(name).contains(&typed) || plain(label).contains(&typed))
}

/// What an `e` prompt's text names: `proc` for an attribute of the selected object, or
/// `Station 3.proc` for an object and an attribute, split at the last dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address<'t> {
    pub object: Option<&'t str>,
    pub attribute: &'t str,
}

pub fn address(text: &str) -> Address<'_> {
    match text.rsplit_once('.') {
        Some((object, attribute)) => Address { object: Some(object.trim()), attribute: attribute.trim() },
        None => Address { object: None, attribute: text.trim() },
    }
}
