# Every attribute can change during a run

Anything a user can set on a class or instance, an attribute, can also be changed while a simulation run is in progress, as an intervention at shown time. Users never see how an object works inside: they change attributes, and later add or change behaviour only at defined hooks. We chose this so that a person or an agent can experiment with a model while it runs, rather than stopping, resetting and running again to see what a change does. The owner chose the trade-off knowingly: every object design now has to define what each of its attributes does when it changes mid-run.

## Considered Options

- **Changes only after Reset,** with a few chosen exceptions such as a Station's process time, as it was until now. Much simpler for every object, but it makes trying a change during a run the exception, and an agent working with a live model would keep having to reset it.

## Consequences

- **The default rule for a change:** what has already started or been scheduled keeps its values, and the change applies from the next decision on. A process time change leaves the MU in work alone, and a new failure profile applies from the next failure. An attribute that can't follow this default gets its own rule when it's built.
- **A change that would leave the model invalid at that moment is refused** with an error, and the value stays as it was. For example, a capacity below the MUs a Station holds right now.
- **A class change during a run** reaches every instance whose inheritance of that attribute isn't cut. Setting a value on an instance cuts its inheritance, and restoring it is its own intervention.
- **Structure is not an attribute.** Adding or removing instances and connections stays after Reset or before Init. Changing structure mid-run is an idea to explore later. Visual attributes, such as name, position, dimensions and look, can change at any time and stay out of the intervention log, because they can't affect results.
- **Performance:** interventions change the run directly and never become events, as `set_process_time` already does, so `ScheduledEvent` stays at 32 bytes and the per-event path is unchanged. A change that must wake something, such as raising a capacity so waiting predecessors retry, schedules ordinary events when it's made. Each attribute's mid-run support is measured with the benchmark as it's built, and this decision is to be looked at again if the cost shows up.
- **Testing:** every attribute gets a test that changes it mid-run, goes back and replays, and checks the results are identical.
