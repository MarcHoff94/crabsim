//! crabsim's UI logic, in plain Rust with no Bevy types, so it can be tested without a window.
//! The binary in `main.rs` draws what this works out, with Bevy and egui.

pub mod attributes;
pub mod camera;
pub mod marks;
pub mod view;
pub mod workspace;
