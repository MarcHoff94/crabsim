# The interface between crabsim and a code module

User code (ADR 0029, spec issue 156) is a WebAssembly module that crabsim's host, `crabsim-code` (`code-host/`), runs with wasmtime behind the engine's `Hooks` trait. This is interface version **1**, built in PR 2. A module built against another version is refused when it loads, with a problem saying which version it has and which crabsim offers.

Users don't write against this interface by hand: the `crabsim-guest` crate (`guest/`) wraps it in Rust, with `hook!` for the functions and types such as `Object`, `Mu`, `Answer` and `Time`. `code-examples/routing` is an example module and `code-examples/routing/every-nth.json` a model using it.

## Why a plain module, not a component

The spec planned a WIT world under the component model, with a plain module and a hand-written interface as the way out if the component model's call cost was too high on the per-MU path. PR 2 took the way out for another reason: **a checkpoint must copy the module's memory** (ADR 0002, ADR 0029), and wasmtime gives no access to a component instance's core memories (`wasmtime::component::Instance` has functions, modules and resources, no memories), so a component couldn't be copied into a checkpoint and going back wouldn't be exact. A plain module's memory is a `wasmtime::Memory`, copied in a few microseconds. Building a component also needs `cargo component` or `wasm-tools` on top of `cargo`, and the `wasm32-wasip2` target links WASI, whose randomness Rust's `HashMap` reads. The cost: the interface is defined here and in `crabsim-guest`, for Rust only until another language's bindings are written by hand, and its numbers are checked by tests rather than by a WIT tool. If wasmtime later lets a component's memory be read, a WIT world can wrap the same functions.

## A module

- **Built for `wasm32-unknown-unknown`**, so it imports nothing but crabsim's functions: no files, network, clock or randomness of its own. A module importing anything else is refused, naming the import. `rust-toolchain.toml` installs the target.
- **Declares its functions** in a custom section `crabsim-functions`, one line each, beginning with the interface version: `1|every_nth(n:Int,left:Object,right:Object,)`. `hook!` writes one line per function, and the linker joins them in an order of its own, so crabsim keeps them by name. A parameter's type is `Int`, `Float`, `Bool`, `Time`, `Object` or `Text`, in any case.
- **Exports each function** as `crabsim:<name>`, taking four `i32`s and returning one, and its memory as `memory`. An `_initialize` export is called once when an instance is made.
- **Keeps its state in its memory** or in globals it exports. A compiler's shadow stack pointer, a global it doesn't export, is back where it began between calls; a module with more than one mutable global it doesn't export is refused, since a checkpoint couldn't copy it.

## A call

A hook function is called with `(kind, object, mu, extra)`:

| `kind & 0xff` | Call | `object` | `mu` | `extra` |
|---|---|---|---|---|
| 0 | on entry | the hook's object | the MU | -1 |
| 1 | on exit | the hook's object | the MU | the object it went to |
| 2 | distribution | the hook's object | the MU | -1 |
| 3 | timer | -1 | -1 | the number given to `call_at` |
| 4 to 7 | before Init, after Init, before Reset, after Reset | -1 | -1 | -1 |

For entry and exit, `kind >> 8` is the trigger point: 1 front, 2 booking point, 3 rear. A distribution answers a successor's id, -1 for default or -2 for hold; any other number ends the run with an error. Other calls' answers are ignored. A hook's parameters are read during the call with `param_int`, `param_float` and `param_text` by their place in the declaration: a bool is 1 or 0, a time milliseconds, an object its id, resolved from its name when the model is built, so a misspelled name is a problem before the run.

## crabsim's functions

Imported from the module `crabsim`. Objects and MUs are ids as `i32`; -1 means none. Texts are written into the module's memory at `ptr`, up to `cap` bytes, and the whole length returned, so the guest asks again with more room when it didn't fit.

- **Time and the model:** `now() -> i64` (ms), `object_count`, `object_kind(o)` (0 Source, 1 Station, 2 Buffer, 3 Drain, 4 Conveyor, 5 cross conveyor), `object_name(o, ptr, cap)`, `find_object(ptr, len)`, `successor_count(o)`, `successor(o, i)`, `predecessor_count(o)`, `predecessor(o, i)`, `process_time(o) -> i64` (-1 if not a Station).
- **The run now:** `object_state(o)` (0 idle, 1 busy or holding, 2 blocked, 3 receiving, 4 failed or stopped, 5 paused), `mus_held(o)`, `content(o, i) -> i64`, `statistic(o, which) -> f64` (0 busy, 1 blocked, 2 idle, 3 failed, 4 paused share, 5 MUs in, 6 MUs out, 7 mean contents, 8 MUs an hour; NaN if none).
- **MUs:** `mu_kind(mu)`, `mu_class_name(kind, ptr, cap)`, `mu_location(mu)`, `mu_left_source(mu) -> i64` (the call's MU only).
- **Parameters:** `param_int(i) -> i64`, `param_float(i) -> f64`, `param_text(i, ptr, cap)`.
- **Random numbers:** `random(o) -> f64` in `[0, 1)`, from the object's own stream for code: the run's seed and the object's stream (ADR 0012), apart from the numbers the object draws itself, so asking changes none of the model's own. Cleared at Reset, copied into checkpoints.
- **Actions,** each answering 1 done or 0 refused, the reason then read with `refusal(ptr, cap)`: `move_mu(mu, to)`, `place_mu(mu, to, at: f64, point)`, `set_process_time(o, ms)`, `set_work_time(o, mu, ms)`, `pause(o, on)`, `stop(o, on)`, `call_at(after_ms, data)`, which calls the same hook function with its parameters again as a timer. Each is the engine's own `HookRun` action, with its rules (PR 1a and 1b in the handoff).
- **Logging and failing:** `log(ptr, len)` gives a line to the run's log sink with its time, object and function, which the CLI writes with `--code-log`. `fail(ptr, len)` gives the message the code panicked with, before it traps; `crabsim-guest` calls it from its panic hook, with the file and line.

Reading a list one item at a time builds the object's state once, kept until the next action.

## The sandbox

- **Fuel:** each call may run `fuel_per_call` instructions, 10 million by default (`Options`), the same on every machine. A call that runs out ends the run with "ran out of fuel", naming the hook, object and MU.
- **Memory:** each instance may take `memory_limit`, 64 MiB by default. Refused more, a Rust module aborts, and the run ends with "ran out of memory".
- **A panic** ends the run with the code's own message: `panicked: <message> at src/lib.rs:12`.
- **Deterministic:** NaNs are canonical and relaxed SIMD deterministic, so a module gives the same results on every machine.
- **No question is asked** before running a model's code, since nothing reaches outside crabsim.

## A run's code

- **One instance per module per run,** made when the run is built, its memory kept between calls.
- **Reset** makes each instance afresh, between the hooks before and after it, so a run after Reset is a fresh run (a test).
- **A checkpoint** is a fresh instance given the copied memory, grown to its size, and the exported mutable globals' values after the last call (a test that fails when the memory isn't copied).
- **Module hashes:** each module's SHA-256 is `Module::hash`, which the CLI prints. Stored runs record them in PR 3.

## Native mode

The same crate compiles natively: `hook!` makes each function a plain Rust function there, and a module of the same name holding its `SIGNATURE`, and `crabsim_guest::native_module!("routing", [routing::every_nth, ...])` registers them in a `Library` with `add_native`. They call the host directly through a thread-local, so a user's own `cargo test` runs the model with them and stops at breakpoints in them. Native code has no sandbox and no fuel; its panics are caught and reported as a WebAssembly module's are. Its state is `with_state`, which the host keeps per run, clears at Reset and copies into checkpoints, so a module written with `with_state` rather than statics behaves the same both ways (a test runs the example module both ways and compares every occurrence and the final state).

## What a call costs

`cargo bench -p crabsim-code --bench calls` runs the demo model for 30 days with a distribution hook at each of its 3 forks. Measured on 2026-11-27 in the cloud container (4 cores), best of 3:

| Scenario | Events/s | Simulated s a wall s | Calls/s | ns a call over no code |
|---|---|---|---|---|
| without code | 12.98M | 26.4M | | |
| `pass` (answers default), WebAssembly | 8.55M | 17.4M | 1.76M | 193 |
| `pass`, native | 8.79M | 17.9M | 1.81M | 178 |
| `cycle` (round robin in code), WebAssembly | 6.76M | 17.8M | 1.35M | 240 |
| `cycle`, native | 7.84M | 20.7M | 1.57M | 138 |

`pass` gives exactly the model's own results, so its difference is the call alone: about 190 ns, two and a half events' worth, where the spec expected about one. Native and WebAssembly differ by only 15 ns, so most of the cost is the engine's path for a code distribution (noting the call, running it after the event, handing the answer back as a `Decided` event and the object's move through the code path) and the host's per-call setup, not wasmtime. `cycle`'s calls cost more, since it reads its successors through imports and keeps a map in its state, and its MUs wait for the successor whose turn it is, which changes the flow. A callgrind of the benchmark over 2 days (all five scenarios in one profile, so the shares are rough) puts the code path's instructions mostly in the engine: `move_by_code` about 105M, the hook checks that every move makes once a model has any hook (`EventContext::note_entry` and `moved_hooked`, about 53M and 52M, though only the forks have hooks) and `run_calls` about 38M, against the host's `Code::call` about 38M and the module's own code too little to show. So the place to make calls cheaper is the engine's path, especially checking only the objects that have hooks, not wasmtime. Not done in PR 2.

## Debugging

Stepping through WebAssembly with lldb, which the spec asked to try once under WSL, was not tried: PR 2 was built in a Linux cloud container, which has lldb but no WSL, and wasmtime here is built without its `debug-builtins` feature and `Config::debug_info`, which lldb needs to find a module's DWARF. Native mode is the way to debug for now; trying lldb is left for PR 4's breakpoints.
