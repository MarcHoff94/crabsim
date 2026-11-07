# AI decisions inside a simulation run

Not built, and not next. Written on 2026-11-07, when AI-native became the second design pillar (ADR 0007).

By default no LLM runs inside a simulation run: agents act from outside, as a user does. The owner expects agents to make real decisions inside material-flow and production systems soon, and wants crabsim to model that. It would be an opt-in: a user who switches it on accepts the cost. Decision models, which are smaller and faster than general LLMs, may make that cost small. This note records what such a feature has to get right, so it can be designed properly later.

## Two kinds of decision

- **A decision at a seam:** the model asks a question at a point where it already applies a rule, such as which successor gets this MU (distribution strategy), which MU leaves next (exit strategy), or later which MU a successor pulls. The default is deterministic code, and an agent is another implementation of the same rule.
- **An agent acting on its own:** a supervisor that looks at the model every so often and acts, for example by pausing a Station for maintenance or changing a process time. These are interventions, so they can reuse the intervention log. The new part is that they happen at simulation times the agent chooses, rather than at shown time.

## Concerns

1. **Reproducibility.** LLM answers can't be reproduced, even at temperature 0, and providers change their models. *Same seed, same result* only survives if every decision is kept in a **decision log**: which object asked, at what simulation time, what it saw and what it was told. Replay, going back, `step_back` and checkpoints then read the log instead of asking again, as they do for interventions. A fresh run without the log isn't reproducible, so its statistics describe that one run, and replications differ in their decisions as well as their seeds. The log should name the model and endpoint that answered.
2. **Reading ahead.** A live session computes ahead of shown time, so it would ask for decisions the user may never see. Going back or an intervention drops the computed future, so those calls are paid for and asked again, maybe with different answers. Read-ahead might stop at the next undecided decision.
3. **Speed.** A call takes from about 0.1 s to several seconds, against a run that does about 13M events per second. A Station that releases one MU a minute and asks once per MU makes about 525,600 decisions per simulated year: at 1 s each, about six days of wall time per simulated year. A local decision model that answers in a millisecond brings that to minutes. This is the figure that decides whether the feature is useful.
4. **The engine must not wait inside an event.** A run is single-threaded, and a live session's thread must keep handling Halt and Reset while a call is out. A decision should therefore end an advance early, with "decision needed", as `advance_bounded` ends early. The caller gets the answer, gives it to the run and advances again. The engine then holds no LLM client and no network code, and knows nothing about where the answer comes from, just as it knows nothing about views.
5. **Simulation time of a decision.** A real agent takes time to decide. The default is that a decision takes no simulation time, like a move, with a configurable latency as an option.
6. **Failure of the endpoint.** A timeout, an error or an answer that isn't one of the allowed choices needs a defined outcome: a run error, or the default rule as a fallback, recorded as such in the log.
7. **What the agent sees.** Each decision needs a description of the state around it, and building that costs time on every decision. What it includes decides how good the decision is. It should be the same typed state the operation catalogue already returns, not something built separately.
8. **Cost and rate limits.** Replications and distributed simulation multiply calls. A run should be able to stop at a budget of calls or tokens, and say so.
9. **Nothing when absent.** Models without AI decisions must not slow down. The seam is a strategy chosen per object, dispatched with a `match`, and the benchmark has to show no change on the per-event path.
10. **User code.** The handoff requires user code to be deterministic and to fit in a checkpoint. AI decisions are the one exception, and the decision log is what restores both properties.

## When to design it

After the MCP server and the in-app agents, because it needs their LLM client, and after the exit and distribution strategy seams from build step 7, because those are where most decisions would plug in.
