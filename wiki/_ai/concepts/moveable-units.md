---
title: Moveable Units (AI context)
for: concepts/moveable-units.md
section: concepts
---

## Facts
- `MoveableUnit { id: u32, location_id: u32, left_source: SimTime }`
- Ids are unique across the model, from `EventContext::new_mu_id`, starting above the highest configured starting MU id
- Source holds the MU it is creating (first one at Init); Station holds at most one (`content: Option<MoveableUnit>`), entrance locked while it does; Drain keeps none
- A Station's starting MU (`add_station_holding`) is part of its default state: worked on from Init, a fresh copy after Reset
- Occurrences along the way: `MuCreated` → `MuMoved` → `WorkStarted` → `WorkFinished` → ... → `MuLeft { lead_time }`
- Lead time runs from leaving the Source to entering the Drain

## Invariants
- Per-MU data exists only through recorded occurrences (ADR 0004); a headless run records nothing, so memory stays flat

## Code locations
- `MoveableUnit` — `src/materialflow_objects/flows.rs`
- `Source::create_mu` — `src/materialflow_objects/source.rs`
- `Drain::process_mu_entrance` — `src/materialflow_objects/drain.rs`
