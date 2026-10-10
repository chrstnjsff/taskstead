# Taskstead

A small Rust task dashboard that runs inside Ghostty.
Create a task, enter its terminal, and return to a list ordered by what needs your attention.

Taskstead is an initial working implementation, not a demonstrated replacement for every Herdr capability.
The research and the narrower product direction are in [the research notes](docs/research.md).

## Run

```sh
brew install chrstnjsff/tap/taskstead
taskstead doctor
taskstead
```

The Homebrew formula installs tmux and Git and builds the Rust binary from the tagged release.
For a local source build:

Requires macOS or Linux, Rust 1.89 or newer, Git, and tmux 3.2 or newer.
Install Codex or Claude Code separately to use those agents.
The shell option works without an AI agent.

```sh
cargo build --release --locked
./target/release/taskstead doctor
./target/release/taskstead
```

Run the dashboard in a Ghostty window.
No changes to Ghostty's settings or keyboard shortcuts are required.
Press `n` to create a task, fill in the project path, choose an agent with the arrow keys, and press Enter.
Isolated tasks start from the repository's current committed HEAD.
Changes in the original checkout are not copied into the new worktree.

```sh
./target/release/taskstead new fix-login --repo /path/to/project --agent codex
./target/release/taskstead new review-api --repo /path/to/project --agent claude
./target/release/taskstead new scratch --repo /path/to/project --agent shell
./target/release/taskstead attach fix-login
./target/release/taskstead diff fix-login
./target/release/taskstead list --json
```

Use `--in-place` to share an existing directory instead of creating a Git worktree.
Custom commands are supported after `--`, with literal arguments and no implicit shell expansion.

```sh
./target/release/taskstead new checks --repo /path/to/project -- cargo test
./target/release/taskstead new dev --repo /path/to/project --in-place -- cargo run
```

## Keys

| Key | Action |
| --- | --- |
| `n` | New task |
| `Enter` | Open the selected terminal |
| `Ctrl+b`, then `d` | Detach from a terminal |
| `j` / `k` or arrows | Select a task |
| `/` | Filter by task, agent, or state |
| `d` | Review changes since the task started |
| `w` | Mark a task as needing input |
| `r` | Clear the attention mark |
| `x` | Stop processes after confirmation |
| `?` | Help |
| `q` | Close the dashboard |

Ghostty keeps its own tabs, splits, copy, and paste shortcuts.
Taskstead shows a read-only terminal preview; Enter connects to the real tmux terminal.
Use tmux's normal copy mode when reviewing longer terminal history.

## Status and persistence

`running` means the process exists.
It does not assert that the agent is thinking, making progress, or waiting for input.
`needs input` and `review` come from explicit reports or manual marks.
Automatic agent-specific hook installation is not included in this version.

Each session receives `TASKSTEAD_SESSION_ID`, `TASKSTEAD_DATA_DIR`, and `TASKSTEAD_BIN` for reporting from a hook or a script.
The `event` command works without a UI process and never sends input to an agent.

```sh
"$TASKSTEAD_BIN" event waiting --detail "Approval required"
"$TASKSTEAD_BIN" event running --detail "Task resumed"
"$TASKSTEAD_BIN" event review --detail "Ready for review"
```

An exit status of zero is shown as `exited`, not as proof that a task is correct or complete.
Nonzero exits are shown as `failed`.
When the runtime disappears, the session is shown as `disconnected`.

Closing Taskstead or Ghostty leaves processes running in tmux.
This does not keep processes running through a reboot, a killed tmux server, or power loss.
Session records and worktrees persist on disk.
`taskstead restart <task>` starts a fresh process in its existing directory; it does not automatically resume an agent conversation.

```sh
./target/release/taskstead stop fix-login
./target/release/taskstead restart fix-login
```

Stopping terminates the selected tmux session and its processes.
It requires confirmation or `--yes`.
Branches and working files are retained.
There is no automatic merge, approval, branch deletion, or worktree cleanup.

## Local state

State lives in `$XDG_STATE_HOME/taskstead`, or `~/.local/state/taskstead` when XDG state storage is not configured.
Use `--data-dir` or `TASKSTEAD_DATA_DIR` to select a different directory.
All clients for a directory share one isolated tmux server and a locked, atomically replaced JSON registry.
Existing tmux sessions and the user's tmux configuration are separate.
The generated `tmux.conf` in the state directory controls this runtime.
In a restricted environment, set `TASKSTEAD_SOCKET` to an absolute socket path inside an existing permitted directory.
Keep the same socket setting for every client using that state directory.
This keeps session communication within the permitted directory without disabling sandbox protections.

Worktree isolation separates Git checkouts, not permissions or processes.
Agents retain their normal permissions and approval behavior.
Retained branches, worktrees, and terminal scrollback consume local disk or memory until explicitly cleaned up.

## Validation

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

The integration tests launch real tmux sessions and Git repositories in temporary directories.
They cover persistence after CLI exit, worktree isolation, exit status, literal arguments, attention reports, confirmation, restart, runtime loss, concurrent clients, and changes committed during a session.
Only test-owned tmux servers and temporary directories are cleaned up.

Native Ghostty verification also exercised opening a task, sending text to its process, detaching, creating a shell task through the form, marking it as needing input, and closing the dashboard without terminating sessions.
