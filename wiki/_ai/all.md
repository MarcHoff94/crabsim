# crabsim — full AI context

Discrete-event material-flow simulator in Rust. Use this file for full project context injection.

---

## CONCEPTS

### Discrete-Event Simulation
- Time advances in discrete jumps event-to-event; nothing happens between events
- `EventController` holds `BinaryHeap<ScheduledEvent>` ordered by (time asc, seq asc)
- Run loop: pop event → advance clock → dispatch to object → schedule returned templates
- `exec_time` in `EventTemplate` is relative; EventController adds `current_time`
- `seq` is monotonically increasing; FIFO order for same-timestamp events depends on it
- `endtime = None` → run until empty; `Some(d)` → stop when `time >= d`

### Material Flow Model
- Three types: `Source` (creates MUs) → `SingleStation` (processes) → `Drain` (consumes)
- Topology is static directed graph wired via `succ`/`pred` before `init()` is called
- Both `succ` and `pred` must be wired consistently — broken pred breaks `BlockedRetry` routing
- `try_to_move_to_successor` uses `RoundRobinStrategy` (starts at `succ_pointer`, wraps)
- `succ_pointer` advances to `(chosen_idx + 1) % n` on successful transfer

### Moveable Units
- `MoveableUnit { id: u32, location_id: u32 }`
- `id` assigned by `Source.mu_id` counter (starts at 1, never reused)
- `SingleStation.content: Option<MoveableUnit>` — max one MU per station
- Drain records `exits: Vec<Duration>` and `exit_ids: Vec<u32>` in arrival order
- FIFO test: `exit_ids[i] == i + 1`
- MU taken via `take_content()` before being passed to `process_mu_entrance`

### Blocked Retry Semantics
- All successors deny → predecessor: `Blocked`, entrance stays locked, recorded in each denied successor's `blockedlist`
- Successor frees up → `notify_blocked_predecessors()` drains `blockedlist`, emits `BlockedRetry` at `exec_time=0`
- `BlockedRetry` handler: check `is_exit_allowed()` first; if false → no-op (station is Processing)
- `MUExit` = scheduled; `BlockedRetry` = reactive — keep them separate
- Always call `notify_blocked_predecessors()` when transitioning to Idle
- **Subtlety**: `MaterialFlow::add_blocked` (trait impl) does NOT dedup; `SingleStation::add_blocked` (inherent) does. Runtime path (`try_to_move_to_successor`) uses trait → no dedup at runtime.

---

## ARCHITECTURE

### Simulator & EventController
- `Simulator.material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>` keyed by object_id
- `register_material_flow(obj)` assigns next id, stores Rc clone, **returns assigned id** — store it
- `add_drain(drain)` also required for drains to appear in `sim.drains` after run
- `EventController.add_event(time, template)` wraps in `ScheduledEvent`, assigns next `seq`
- `run(simulator)` consumes simulator, returns it after completion
- `seq` must stay monotonically increasing — deterministic tests depend on it

### Material Flow Objects
- `EventInterface`: `init`, `execute_event`, id accessors
- `MaterialFlow: EventInterface`: entrance/exit gates, `process_mu_entrance/exit`, `add_blocked`, `notify_blocked_predecessors`
- `SuccessorContainer`: `successors`, `succ_pointer`, `take_content`, `clear_content_and_idle`
- **Source**: `init()` creates first MU immediately. `is_entrance_allowed()=false`. `add_blocked` panics.
- **SingleStation**: `state: Idle|Processing|Blocked`. `is_entrance_allowed()` = `!entrancelocked && state==Idle && interruption==None`. `is_exit_allowed()` = `!exitlocked && (Idle||Blocked) && interruption==None`.
- **Drain**: always accepts (`entrance_locked` starts false). `notify_blocked_predecessors` = no-op. No scheduled events.
- `FailureProfile`: `availability=1.0` → inactive (fraction, not percentage). Otherwise calculates `mtbf` from `mttr/(1/availability - 1)` using f64 arithmetic.

### Visualization
- Topology printed once; state snapshot after each event
- External renderer: TCP + newline-delimited JSON `{"kind":"topology"/"snapshot",...}`
- `speed = N` → 1 sim-second = 1/N wall-seconds; `speed = -1` → unpaced

---

## REFERENCE

### Event Types
- `MUExit`: processing complete → `try_to_move_to_successor` → Idle+notify OR Blocked
- `BlockedRetry`: successor freed → retry transfer (no-op if Processing)
- `MUEntrance`, `Finished`, `StartFailure`, `EndFailure`: defined but not actively dispatched

### Key file locations
- Traits + helpers: `src/materialflow_objects/flows.rs`
- `Source`: `src/materialflow_objects/source.rs`
- `SingleStation`: `src/materialflow_objects/station.rs`
- `Drain`: `src/materialflow_objects/drain.rs`
- `Simulator`, `EventController`: `src/simulator.rs`
- `EventType`, `EventTemplate`, `ScheduledEvent`: `src/event.rs`
- `Distribution`, `DistributionSampler`: `src/random_numbers.rs`
- Public API + tests: `src/lib.rs`
- CLI: `src/main.rs`

### Tests that must not regress
`drain_fifo_order`, `ordering_across_stations_preserved`, `ordering_across_parallel_stations_preserved`, `station_becomes_blocked_when_no_successor_accepts`, `blocked_retry_while_predecessor_processing_does_not_move_mu`, `blocked_retry_full_flow`, `blockedlist_dedup`, `source_continues_producing_after_blocked_retry`

### CLI / env vars
- `--inplace` / `CRABSIM_LIVE_INPLACE=1` — in-place ANSI rendering
- `--renderer-window` / `CRABSIM_RENDERER=1` — external renderer
- `--speed <f>` / `CRABSIM_RENDERER_SPEED=<f>` — pacing (default 5; -1=unpaced)
- `RUST_LOG=info|debug` — log level
- Endtime hardcoded at 3600s in `src/lib.rs::run_demo`
