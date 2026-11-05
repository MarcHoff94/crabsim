---
title: Adding a New Station Type
section: guides
related:
  - material-flow-objects
  - blocked-retry
  - writing-tests
---

# Adding a New Station Type

This guide walks through adding a new type of material flow object — for example, a station that buffers multiple MUs, or one with variable processing time.

## Step 1: Define the struct

Your struct needs a `BaseObject` and fields for whatever state it tracks.

```rust
pub struct BufferStation {
    pub base: BaseObject,
    pub capacity: usize,
    pub queue: VecDeque<MoveableUnit>,
    pub succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub succ_pointer: usize,
    pub blockedlist: Vec<u32>,
    pub proc_time: Duration,
}
```

## Step 2: Implement EventInterface

```rust
impl EventInterface for BufferStation {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        Ok(Vec::new())  // no initial events unless you pre-load MUs
    }

    fn execute_event(
        &mut self,
        simulator: &mut Simulator,
        eventtype: &EventType,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match eventtype {
            EventType::MUExit => { /* move front of queue to successor */ }
            EventType::BlockedRetry => { /* retry if exit allowed */ }
            _ => return Err(RunTimeError {}),
        }
    }

    fn get_object_id(&self) -> &u32 { &self.base.object_id }
    fn set_object_id(&mut self, id: u32) { self.base.object_id = id; }
    fn set_random_seed(&mut self, seed: u64) { self.base.random_seed = seed; }
    fn as_any(&self) -> &dyn std::any::Any { self }
}
```

## Step 3: Implement MaterialFlow

```rust
impl MaterialFlow for BufferStation {
    fn is_entrance_allowed(&self) -> bool {
        self.queue.len() < self.capacity
    }
    fn is_exit_allowed(&self) -> bool {
        !self.queue.is_empty()
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        self.queue.push_back(mu);
        // schedule processing if first item
        if self.queue.len() == 1 {
            return Ok(Some(EventTemplate {
                eventtype: EventType::MUExit,
                exec_time: self.proc_time,
                object_id: self.base.object_id,
                seq: 0,
            }));
        }
        Ok(None)
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> { Ok(None) }
    fn add_blocked(&mut self, id: u32) { self.blockedlist.push(id); }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        let events: Vec<_> = self.blockedlist.drain(..)
            .map(|id| EventTemplate { eventtype: EventType::BlockedRetry, exec_time: Duration::ZERO, object_id: id, seq: 0 })
            .collect();
        Ok(if events.is_empty() { None } else { Some(events) })
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any { self }
}
```

## Step 4: Implement SuccessorContainer

Required if your type has successors (so `try_to_move_to_successor` can use it).

```rust
impl SuccessorContainer for BufferStation {
    fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>> { &self.succ }
    fn succ_pointer(&self) -> usize { self.succ_pointer }
    fn succ_pointer_mut(&mut self) -> &mut usize { &mut self.succ_pointer }
    fn take_content(&mut self) -> Option<MoveableUnit> { self.queue.pop_front() }
    fn clear_content_and_idle(&mut self) { self.queue.clear(); }
}
```

## Step 5: Expose in mod.rs

Add a `pub use` in `src/materialflow_objects/mod.rs`:

```rust
pub use crate::materialflow_objects::buffer_station::BufferStation;
```

## Step 6: Register and wire

```rust
let buf = Rc::new(RefCell::new(BufferStation::new(...)));
simulator.register_material_flow(buf.clone());

// wire successors and predecessors
buf.borrow_mut().succ = vec![next_station.clone()];
next_station.borrow_mut().pred = vec![buf.clone()];
```

## Rules to preserve

- Call `notify_blocked_predecessors()` whenever your object frees up space, otherwise blocked predecessors will never be woken.
- In `BlockedRetry`, check `is_exit_allowed()` first and return `Ok(None)` if `false` — this prevents moving MUs out of an object that hasn't finished processing.
- Keep `seq = 0` in returned `EventTemplate`s — the `EventController` overwrites it with the real sequence number at scheduling time.
