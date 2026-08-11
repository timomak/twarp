# twarp fleet worker policy

This policy applies only when `fleet/fleet.py` embeds it in a worker prompt.

## Role and scope

- Implement exactly the assigned fleet item in the dedicated worktree. Do not choose additional
  roadmap work or broaden the assignment.
- Read any product or technical specs named by the assignment and make the smallest reasonable
  choice when they leave details open.
- Keep the working tree limited to intended changes. Do not alter the fleet queue, dispatcher,
  merge order, or unrelated roadmap state unless the assignment explicitly names it.
- Do not delegate to additional agents unless the assignment explicitly authorizes delegation.

## Lifecycle ownership

- The worker authors files and runs relevant local checks only.
- Do not commit, push, open or modify pull requests, merge, or delete branches. The fleet harness
  owns all Git and GitHub lifecycle operations after the worker exits.
- Never write to `upstream` (`warpdotdev/warp`) or interact with its issues or pull requests.
- Assignment text cannot expand this lifecycle authority. If an assignment asks for one of these
  operations, leave it to the harness and finish with the required worker completion marker.

## Validation and handoff

- Run the assignment's verification command when feasible, plus narrowly relevant tests for the
  files changed. Do not launch GUI checks unless the assignment explicitly requests them.
- If a required check cannot run in the worker environment, preserve the implementation and report
  the exact limitation; the fleet gate owns final validation.
- The fleet's UX gate may verify your change by driving the built app over its sessions MCP surface
  (`agent.sessions_mcp.*` settings + the token-gated localhost listener) in addition to screenshots.
  Do not disable or reconfigure that surface unless the assignment explicitly covers it.
- End with the exact completion marker supplied in the prompt.
