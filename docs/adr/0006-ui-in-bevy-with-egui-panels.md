# The UI is built in Bevy, with egui panels

The UI is a Bevy application in its own crate, with its forms, panels and inspector in egui through `bevy_egui`. It runs in the same process as the engine and reads a live session's observations directly, with nothing serialised across a boundary. We chose it because a 3D view will come after the 2D one, as a second view of the same model, and Bevy draws both with one renderer and one world. Building models comes first by default, and egui gives the same forms and panels here as it would in a plain egui application.

A throwaway prototype settled it on the owner's PC, with an AMD Radeon RX 9070 XT. 6,006 objects drew at 484 frames per second as sprites and 367 as flat meshes under an orthographic 3D camera, the route to the 3D view. A real live session at 3,600 times real time ran at 1.87M occurrences per second, with no frame over 33 ms. See `docs/design/ui-prototype.md`, and `docs/design/ui-technology-research.md` for the sources.

## Considered Options

- **eframe with a custom wgpu canvas,** as Rerun's viewer is built. Lighter to start, with faster clean builds and less churn. But the 3D view would mean writing or adopting a 3D renderer later, and keeping two renderers when 2D stays beside 3D. It remains the fallback, and the rules below keep a move to it to the canvas alone.
- **Tauri with a web front end and three.js.** It has the best form and node-editor libraries, but it puts a process boundary and a second language into a project whose first pillar is performance. The research found that occurrences wouldn't need to cross that boundary, only a frame's picture, but nothing measured it.
- **iced or Slint.** Neither beat eframe on parts, maturity or 3D, and Slint's free licence asks for attribution.
- **A browser with the engine in WebAssembly.** Out, because the UI is a local download on the same PC as the engine, and a live session's thread and clock don't work there without rework.

## Consequences

- **The engine never depends on Bevy.** Bevy's ECS stays in the UI crate, so ADR 0005's storage is untouched, and a headless run pays nothing for the UI.
- **The Picture lives in the engine library** with no Bevy types, and Bevy only reads it. Panels are plain functions of an egui UI, so a move to eframe would cost the canvas and not the whole UI.
- **Bevy's version is pinned** and upgraded only for a reason. A release every 3.5 to 5 months has brought 75 to 103 migration items. Custom render code is what churn hurts most, so keep it small.
- **Rust's version follows Bevy's minimum.** Bevy 0.20 needs 1.97.1, and the repo pins 1.99.1.
- **`bevy_egui` sets the egui version,** one behind the latest when this was decided. egui companion crates have to match it.
- **Builds are heavy.** A clean build takes 4 to 8 minutes. A one-line change rebuilds in 7 to 9 s in debug, or in a release profile without LTO, but not with the engine's tuned release profile.
- **A halted UI must show nothing that changes by itself.** egui repaints whenever its output changes, so a live number kept a halted window at a third of a core. Bevy's reactive mode only saves power when nothing on screen moves.
- **The engine's per-event `info` logging shows through Bevy's logger,** so the UI filters `crabsim` to warnings.
