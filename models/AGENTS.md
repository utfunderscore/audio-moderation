# Project Agent Instructions

## Modal deployment approval gate

Never execute a Modal command that can create, update, start, delete, or incur cost for
remote resources without the user's explicit approval immediately before execution.

## No runaway agent spawning

If the user requests a large task consisting of multiple diferrent stages, review and confirm the user before contuously spawning more agents. 
After each goal is reached, there should be a summary written and next steps to be confirmed.
