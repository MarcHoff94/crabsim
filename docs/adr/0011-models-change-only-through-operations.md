# Models change only through crabsim's operations, and the user controls who edits

**Partly superseded by ADR 0013** (2026-11-09): there is no edit queue, a claim is granted or refused at once, and every change an agent makes needs a claim. The rest stands: models change only through crabsim's operations, the user controls who edits, and one person's front-end works on a models folder.

Model files stay readable JSON that people, agents and git can read and diff, but they change only through crabsim's operations: in the UI, which is the user's main way, in the CLI, or through an agent's harness over MCP. This replaces one point of ADR 0007, which chose files that agents write with their own file tools. We changed it because several agents and a person working on one models folder need to know who is changing a model, and a file written past crabsim can't take part in that. Its old objections no longer hold: `edit_model` applies a batch of changes in one call, so building through operations isn't chatty, and the files are still in git.

Only one editor can change a model file at a time. The user can take the editor right at any time, and decides who comes next. Agents claim the right, stating their purpose, and wait in an edit queue, first come, first served unless the user chooses otherwise. Only one person's front-end works on a models folder: one UI window, or the CLI when no UI is open. A second window means an independent copy of the folder.

## Considered Options

- **Files that anyone writes directly, as ADR 0007 chose.** Simplest, and any tool can edit a model, but two writers lose each other's changes without anyone noticing, and nobody can tell who changed what.
- **A hash check alone:** every change names the hash of the file it read, and is refused if the file changed since. It catches a stale write but doesn't say who may change a model, so two agents would keep refusing each other.
- **One equal queue for everyone,** people and agents, with no priority. Simpler, but a person would wait for an agent in the middle of 30 calls, and couldn't stop one going wrong.

## Consequences

- **The first deliberate exception to the AI-native pillar:** controlling the edit queue (taking the right, giving it, choosing who is next, removing or blocking an agent) is possible in the UI and the CLI, never over MCP, because an agent able to take the right from the user would defeat the purpose. Over MCP an agent can only claim, release and see the queue. Giving that control to a built-in crabsim agent that coordinates other agents, as an opt-in by the user, is a possible later feature that would bring this back in line with the pillar.
- **crabsim can't stop other programs writing a file.** A text editor, a harness's own file tools or `git checkout` still can. crabsim records each file's hash as it last wrote it, and refuses to change a file that differs, or that it has no record of, until it is adopted. Reads and runs still work, and say the file changed outside crabsim. The setup notes for each harness say how to deny its own file tools on the models folder.
- **The cost:** each change loads, checks and saves the whole file, measured on the 6,006-object benchmark model when it's built. Nothing touches a simulation run or the per-event path.
- **An agent waiting for the right polls** `claim_model`, as it polls `run_status`. Notifying it instead is an open question for later.
