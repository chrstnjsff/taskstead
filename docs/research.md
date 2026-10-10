# Research and product direction

Researched on 2026-10-10 using the user's DeepAPI skill.
Five separate web searches were followed by primary documentation, public GitHub issues, and one DeepAPI deep research request.
Context7 supplied the Ghostty, Ratatui, and tmux implementation documentation.

## What Herdr already does

[Herdr's official site](https://herdr.dev/) and [agent documentation](https://herdr.dev/docs/agents/) describe persistent terminals, agent status, multiple providers, workspaces, and background sessions.
Its [CLI reference](https://herdr.dev/docs/cli-reference/) also exposes worktree and agent workflows.
Rust, Ghostty compatibility, worktrees, and persistent sessions are therefore not unique selling points for this project.

The current agent documentation explicitly describes screen and terminal-title detection, fallback states, integration reports, and `unknown` behavior for Codex.
Some agents already report state through integrations.
A claim that Herdr uses only heuristic status detection would be inaccurate.

## Evidence for a narrower workflow

[Ghostty as a Shell for Herdr](https://suraj.io/post/2026/ghostty-as-a-shell-for-herdr/) describes collisions between Ghostty's native Command shortcuts and Herdr shortcuts, along with a detailed remapping setup.
That is evidence for reducing shortcut overlap, not evidence that every Herdr user experiences those problems.
Taskstead's dashboard uses ordinary keys while focused and leaves Ghostty's native shortcuts alone.

Public Herdr issue reports include [agent-state misclassification](https://github.com/herdrdev/herdr/issues/4573), [incorrect state during a Kimi status line](https://github.com/herdrdev/herdr/issues/1631), [Shift+Enter behavior under Ghostty](https://github.com/herdrdev/herdr/issues/1844), and [resource usage with several agents](https://github.com/herdrdev/herdr/issues/3217).
These are user reports, not independently reproduced measurements.
They support testing input compatibility and being precise about status provenance.
They do not establish that Taskstead is faster, more reliable, or universally better.

## Chosen first version

The product unit is a task: one name, one working directory, one terminal process.
The main loop is create, open, return, and review.
The dashboard sorts explicit attention states first and includes a preview and a diff from the task's initial commit.
There is no workspace-tab-pane hierarchy in the Taskstead interface.

All application code is Rust, using Clap for the CLI, Ratatui and Crossterm for the UI, and Serde for local state.
Tmux is an external runtime dependency written outside this project, not a Rust component.
It owns PTYs, input forwarding, terminal resizing, scrollback, and detach/reattach behavior.
Using it avoids implementing a second terminal emulator and custom PTY daemon in the initial version.
This does not guarantee perfect forwarding of every Ghostty or agent terminal feature.

The registry uses an exclusive OS file lock and atomic replacement.
Git supplies isolated worktrees from HEAD.
Commands are passed as literal arguments, including protection against tmux's command separator parsing.
No agent is auto-approved, and no working tree is automatically merged or deleted.

## Acceptance criteria

- A new task can be created from the dashboard or CLI.
- Codex, Claude Code, shells, and custom commands can run as persistent terminal processes.
- Closing a dashboard does not terminate its task processes.
- An isolated task can edit files without modifying the original checkout.
- Explicit attention reports survive separate CLI invocations and sort before ordinary running tasks.
- Exit codes and lost runtimes cannot silently become successful task completion.
- Review includes changes committed after task creation and identifies untracked files separately.
- Concurrent clients cannot overwrite each other's task records.
- Stopping requires confirmation and retains working files.
- Keyboard navigation and attachment work in a real terminal.

## Boundaries and next evidence

This version does not install agent hooks, infer blocked prompts, resume conversations after reboot, coordinate remote machines, or offer a plugin marketplace.
The next product decision should follow use of this small workflow with real tasks.
Measure time to create and reopen a task, missed attention requests, input regressions, idle CPU, and memory with identical workloads in Herdr and Taskstead.
No comparative benchmark or user validation has been completed.

## DeepAPI request provenance

| Request | Status | Evidence |
| --- | --- | --- |
| `fa1a9e8f-7183-409a-8995-ea4cdff62fd5` | succeeded | Herdr and Ghostty discovery |
| `be87cee8-7674-494e-a7e3-0aa67b1bdfea` | succeeded | Existing feature set |
| `adf5eab7-c921-4ac5-b396-58c7b45826fb` | succeeded | Worktrees and orchestration |
| `36be7dc4-28e3-4120-a8cb-dd59286425ef` | succeeded | Companion alternatives |
| `c6df7771-e69f-4cfa-814d-723beb80b85f` | succeeded | Reported limitations |
| `f147423e-70ce-4912-965e-0f3222b3841c` | succeeded, complete | Primary-source research and architecture tradeoffs |
| `c6a738e7-11c3-4a95-a38d-87e651343744` | succeeded | All four requested website sources returned content |
