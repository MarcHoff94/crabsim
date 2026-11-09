# Agents claim a model file, with no edit queue

An agent that wants to change a model file claims it, saying its goal and what it is doing now. The claim is granted at once if nobody holds the file, and refused otherwise, naming who holds it, with which harness, for what goal and since when. There is no edit queue. Every change an agent makes needs a claim, including creating and deleting a file. The agent updates what it is doing as it goes, and the user sees every claim in the UI's Editing tab, including those made while the window was closed. This supersedes ADR 0011's edit queue and its rule that a single change to a free file needs no claim. Models still change only through crabsim's operations, and the user still controls who edits.

We changed it after the owner tried the Editing tab with a real agent on 2026-11-09. The agent never appeared, because it only made single changes, which needed no claim. Thinking about the fix, the owner found the queue solves the wrong problem: it orders agents one after another, but agent 2 can still undo agent 1's work in its next change. Nothing in a queue prevents that, so it adds machinery without adding safety. What the owner wants is a simple tool to see and manage the agents working at the time.

## The decisions

- **A claim carries `goal` and `doing`,** both required. `goal` replaces `purpose`. Claiming a file it already holds updates both and renews the claim, so there is no new operation. The last 10 values of `doing` are kept with the claim, newest first, and cleared when it ends.
- **A claim ends** when the agent releases it, when its process ends, after 10 minutes without a change or a claim, or when the user revokes it.
- **Taking the right pauses a claim:** the agent's changes are refused as paused, and its idle time stops counting. When the user releases the right, or the window holding it closes, the claim goes back to the agent.
- **An agent reads again after anyone else's change:** if anyone but the agent changed the file since its last change, its next change is refused once, telling it to read the model again. This covers a paused claim and the user's one-change take-overs ("Change anyway", "Move anyway", the CLI's `--take-over`). crabsim already keeps the file's hash after each change it makes, so this costs nothing.
- **The user's right ends when the UI window closes,** so a closed window never locks agents out. A CLI command's take-over lasts that one command.
- **The user's controls** are take, release, revoke, block and unblock: in the UI's Editing tab, and in the CLI's `crabsim editors` (show, revoke, block, unblock), never over MCP.

## Considered Options

- **Keep the edit queue,** with one change at a time ordered by it. Agents still undo each other one after another, and waiting agents poll for their turn.
- **A plan of steps with each claim,** and a stated duration capped by the user. More ceremony for agents, and a plan written up front goes stale. A goal plus what the agent is doing now stays current and costs one call to update.
- **Losing the claim when the user takes the right.** Simpler, but another agent could claim the file between the user's change and the first agent's return. Pausing keeps the claim, and the refusal after anyone else's change gives the same protection against a stale picture.
- **The user approving each claim.** Full control, but an agent stalls whenever the user is away from the window.

## Consequences

- `give_model`, `move_in_queue` and `remove_from_queue` go, `get_edit_queue` becomes `get_editors`, and agents no longer poll for a turn. The three removed operations were the user's own and never offered over MCP, so the MCP tool list gains nothing and keeps its size (issue 58).
- An agent that only wants one change makes three calls: claim, change, release. That is the price of being visible.
- Agents can still undo each other's work one after another. Seeing who works on what is the user's tool against that. An edit log, which would show it afterwards, is a possible later feature.
- `.crabsim/editing.json` files written before this are read with their queues dropped.
