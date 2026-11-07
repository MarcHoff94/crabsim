# Model files live in their own crate, and the engine knows nothing of them

Model files, their validation, classes and the JSON Schema live in a workspace crate of their own, `crabsim-model`, which turns a file into the engine's existing `ModelDescription`. The engine gains no dependency at all, not even `serde`, which issue 28 had allowed it. We chose this because the schema must be generated from the very types that read the file, so if those types lived in the engine it would need `schemars` as well as `serde`, and the engine would carry a file format and classes that a headless run never uses. A run is built from a `ModelDescription` exactly as before, so the per-event path and a headless run pay nothing for files or classes.

## Considered Options

- **The file types in the engine, with `serde` and `schemars`.** One crate fewer, and the types could sit next to `ModelDescription`. But the engine would then own a file format, class resolution and schema generation, none of which a run needs, and every change to the format would touch the engine.

## Consequences

- **The CLI moves to its own crate,** `cli/` (package `crabsim-cli`), keeping the binary name `crabsim`. The engine's own binary couldn't read a model file, because Cargo forbids the loop of the engine's binary depending on `crabsim-model`, which depends on the engine. `clap` and `env_logger` move with it, so the engine sheds them.
- **`crabsim-model` and `crabsim-cli` are default members** of the workspace, so `cargo test` and `cargo run --bin crabsim` at the root keep working and test them. They add only `serde`, `serde_json` and `clap` to a build. The UI stays a non-default member, so the root still never builds Bevy.
- **The operation catalogue and the MCP server,** when they come, depend on `crabsim-model`, as the UI will once it builds models.
- **A `ModelDescription` can't be saved as a file.** Files are built from `crabsim-model`'s own types, and a `ModelDescription` is what they build, not the other way round. Code that needs a model as a file, such as the demo, has it as a file.
