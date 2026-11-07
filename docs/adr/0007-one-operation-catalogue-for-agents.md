# Agents use crabsim through one operation catalogue, with MCP as the first front-end

crabsim is AI-native: anything a person can do, an AI agent can do too. Every capability is defined once, in Rust, as an operation with a name, a typed input, a typed output and a description: validate a model, do a headless run, run replications, read statistics, and later control a live session. Front-ends offer the catalogue to agents. The first is an MCP server, so that an external harness such as Claude Code can build, run and manage models. The second, later, is agents inside the crabsim application that call the user's own LLM endpoint. We chose one catalogue so the two front-ends can't drift apart, and so the in-app agents only add an agent loop and an LLM client on top of what the MCP server already proved.

## Considered Options

- **An MCP server written directly against the engine, with in-app agents built separately later.** Faster to start, but two descriptions of every capability would drift, and the in-app agents would repeat the work.
- **Building models only through MCP calls,** such as one `add_station` per object. Chatty, slow to review and invisible to git. Models are readable text files instead, which an agent can write with its own file tools, and operations validate, run and read them. Changing a model through operations stays wanted for later, for an open UI or a large model where rebuilding from the file would cost too much.
- **The MCP server inside the UI application** from the start, so the user watches the agent work on the model on screen. It's wanted later, but it would tie the server to Bevy and the window's lifecycle. It can come later as another front-end on the same catalogue.

## Consequences

- **The MCP server is its own crate,** a non-default workspace member started over stdio, with no window. The engine never depends on it, as with the UI, so `cargo test` and `cargo bench` at the root never build it, and a headless run pays nothing.
- **Models become data first.** The model file format and validation, which were left for later, come before the MCP server, because an agent can't build what can't be written down. The engine gets `serde` back, used only by the model description, so the per-event path is unchanged.
- **No LLM runs inside a simulation run** by default. Agents act from outside, as a user does: they change model files, start runs, make interventions and read results. AI decisions inside a run are a possible opt-in for later, designed in `docs/design/ai-decisions-in-runs.md`.
- **Agents retrieve context through graph operations** on the model in memory, such as upstream, downstream and paths, with `describe_model` as an overview, rather than reading a whole model file. An embedded graph database was left out until these prove too limited.
- **Every new capability comes with its operation.** A feature reachable only from the UI is a gap, not a choice.
