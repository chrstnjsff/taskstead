use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::ExitCode,
};
use taskstead::{
    git,
    model::{Agent, State, clean},
    runtime::executable,
    service::{NewSession, Service},
    store::Store,
    ui,
};

#[derive(Parser)]
#[command(
    version,
    about = "Persistent coding sessions, one simple dashboard. Run inside Ghostty."
)]
struct Cli {
    #[arg(long, global = true, env = "TASKSTEAD_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    #[command(about = "Start an isolated task; its process survives closing the dashboard")]
    New {
        name: String,
        #[arg(long, short, default_value = ".")]
        repo: PathBuf,
        #[arg(long, short, value_enum, default_value = "codex")]
        agent: Agent,
        #[arg(
            long,
            help = "Use the project directory directly instead of creating a worktree"
        )]
        in_place: bool,
        #[arg(long, help = "Immediately attach to the new session")]
        attach: bool,
        #[arg(
            last = true,
            help = "Custom command and arguments, passed without shell expansion"
        )]
        command: Vec<String>,
    },
    #[command(about = "List sessions, with those needing attention first")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Enter a session; Ctrl+b then d returns to your terminal")]
    Attach { session: String },
    #[command(about = "Show changes since the task started, including committed changes")]
    Diff { session: String },
    #[command(about = "Print recent terminal output")]
    Logs { session: String },
    #[command(about = "Report an explicit agent state, from a hook or your own command")]
    Event {
        #[arg(value_enum)]
        state: State,
        #[arg(long, env = "TASKSTEAD_SESSION_ID")]
        session: String,
        #[arg(long, default_value = "")]
        detail: String,
    },
    #[command(about = "Stop a session's processes; retain all branches and files")]
    Stop {
        session: String,
        #[arg(long, short)]
        yes: bool,
    },
    #[command(about = "Start a fresh agent process in a stopped session's existing directory")]
    Restart { session: String },
    #[command(about = "Check dependencies and show local state paths")]
    Doctor,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("taskstead: {}", clean(&format!("{error:#}")));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = match cli.data_dir {
        Some(path) => path,
        None => Store::default_root()?,
    };
    let service = Service::open(root)?;
    match cli.command {
        None => ui::run(&service)?,
        Some(Action::New {
            name,
            repo,
            agent,
            in_place,
            attach,
            command,
        }) => {
            let session = service.create(NewSession {
                name,
                directory: repo,
                command: if command.is_empty() {
                    agent.command()
                } else {
                    command
                },
                in_place,
            })?;
            println!(
                "{}  {}\n{}\nAttach: taskstead attach {}",
                session.name,
                session.id,
                session.directory.display(),
                session.name
            );
            if attach {
                service.runtime.attach(&session)?;
            }
        }
        Some(Action::List { json }) => {
            let sessions = service.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else if sessions.is_empty() {
                println!("No sessions yet. Run taskstead new <name> --repo <project>.");
            } else {
                println!("{:<10} {:<22} {:<14} AGENT", "ID", "TASK", "STATE");
                for session in sessions {
                    println!(
                        "{:<10} {:<22} {:<14} {}",
                        &session.id[..8],
                        clean(&session.name),
                        session.state.to_string(),
                        clean(&session.agent)
                    );
                }
            }
        }
        Some(Action::Attach { session }) => service.runtime.attach(&service.get(&session)?)?,
        Some(Action::Diff { session }) => {
            let session = service.get(&session)?;
            print!(
                "{}",
                clean(&git::diff(&session.directory, session.base.as_deref())?)
            );
        }
        Some(Action::Logs { session }) => print!(
            "{}",
            clean(&service.runtime.capture(&service.get(&session)?)?)
        ),
        Some(Action::Event {
            state,
            session,
            detail,
        }) => service.event(&session, state, detail)?,
        Some(Action::Stop { session, yes }) => {
            let item = service.get(&session)?;
            if !yes {
                if !io::stdin().is_terminal() {
                    bail!("Stopping terminates the session's processes; pass --yes to confirm");
                }
                print!(
                    "Stop {} and its processes? Files will be kept. [y/N] ",
                    clean(&item.name)
                );
                io::stdout().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    return Ok(());
                }
            }
            service.stop(&session)?;
            println!(
                "Stopped {}. Files retained in {}",
                item.name,
                item.directory.display()
            );
        }
        Some(Action::Restart { session }) => {
            let item = service.restart(&session)?;
            println!(
                "Started a fresh process for {}. Attach: taskstead attach {}",
                item.name, item.name
            );
        }
        Some(Action::Doctor) => {
            let mut missing = false;
            for (name, required) in [
                ("tmux", true),
                ("git", true),
                ("codex", false),
                ("claude", false),
            ] {
                match executable(name) {
                    Ok(path) => println!("OK       {name}: {}", path.display()),
                    Err(_) => {
                        println!(
                            "MISSING  {name}{}",
                            if required {
                                " (required)"
                            } else {
                                " (optional)"
                            }
                        );
                        missing |= required;
                    }
                }
            }
            println!(
                "State:   {}\nRuntime: {}\nTerminal: {}",
                service.store.root.display(),
                service.runtime.location(),
                std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".into())
            );
            service.store.lock().context("Registry check failed")?;
            println!("Sessions survive terminal closure while tmux runs. Reboots stop processes.");
            if missing {
                bail!("Install the missing required dependencies");
            }
        }
    }
    Ok(())
}
