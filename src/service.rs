use crate::{
    git,
    model::{Session, State, now},
    runtime::{Runtime, executable},
    store::{LockedStore, Store},
};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub struct Service {
    pub store: Store,
    pub runtime: Runtime,
}

pub struct NewSession {
    pub name: String,
    pub directory: PathBuf,
    pub command: Vec<String>,
    pub in_place: bool,
}

impl Service {
    pub fn open(directory: PathBuf) -> Result<Self> {
        let store = Store::open(directory)?;
        let runtime = Runtime::new(&store)?;
        Ok(Self { store, runtime })
    }

    fn refresh(&self, registry: &mut LockedStore) -> Result<()> {
        if registry.sessions.iter().all(|s| !s.state.active()) {
            return Ok(());
        }
        let panes = self.runtime.panes()?;
        let mut changed = false;
        for session in &mut registry.sessions {
            if !session.state.active() {
                continue;
            }
            match panes.get(&session.target()) {
                Some(pane) if pane.dead => {
                    session.exit_code = pane.exit_code;
                    let state = if pane.exit_code == Some(0) {
                        State::Exited
                    } else {
                        State::Failed
                    };
                    session.set_state(
                        state,
                        format!(
                            "Process exited with status {}. Worktree retained.",
                            pane.exit_code.map_or("unknown".into(), |n| n.to_string())
                        ),
                    );
                    changed = true;
                }
                None => {
                    session.set_state(
                        State::Lost,
                        "The terminal process is gone. Files are retained; restart is explicit.",
                    );
                    changed = true;
                }
                Some(_) if session.state == State::Starting => {
                    session.set_state(
                        State::Running,
                        "Process alive; agent turn status is unknown.",
                    );
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            registry.save()?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        let mut registry = self.store.lock()?;
        self.refresh(&mut registry)?;
        registry.sessions.sort_by_key(|s| {
            (
                s.state.priority(),
                std::cmp::Reverse(s.updated),
                s.name.clone(),
            )
        });
        Ok(registry.sessions)
    }

    pub fn get(&self, query: &str) -> Result<Session> {
        let mut registry = self.store.lock()?;
        self.refresh(&mut registry)?;
        Ok(registry.sessions[registry.find(query)?].clone())
    }

    pub fn create(&self, input: NewSession) -> Result<Session> {
        validate_name(&input.name)?;
        let mut command = input.command;
        let program = command.first().context("A command is required")?;
        let path = executable(program)?;
        executable("tmux")?;
        let agent = path
            .file_name()
            .context("Executable has no file name")?
            .to_string_lossy()
            .into_owned();
        command[0] = path
            .to_str()
            .context("Executable path is not UTF-8")?
            .into();
        let directory = input
            .directory
            .canonicalize()
            .context("Project directory does not exist")?;
        if !directory.is_dir() {
            bail!("Project path is not a directory");
        }
        let repo = if input.in_place {
            directory.clone()
        } else {
            git::root(&directory).context("An isolated session requires a Git repository; use --in-place for a plain directory")?
        };
        let base = if input.in_place {
            git::head(&repo).ok()
        } else {
            Some(
                git::head(&repo)
                    .context("Commit your initial files before creating an isolated session")?,
            )
        };
        let mut registry = self.store.lock()?;
        if registry.sessions.iter().any(|s| s.name == input.name) {
            bail!(
                "A session named {:?} already exists; choose a different name",
                input.name
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        let branch = (!input.in_place).then(|| format!("taskstead/{}-{}", input.name, &id[..8]));
        let directory = if input.in_place {
            directory
        } else {
            self.store.root.join("worktrees").join(&id)
        };
        let mut session = Session {
            id,
            name: input.name,
            agent,
            repo,
            directory,
            branch,
            base,
            command,
            state: State::Starting,
            detail: "Preparing session".into(),
            created: now(),
            updated: now(),
            exit_code: None,
        };
        registry.sessions.push(session.clone());
        registry.save()?;
        let result = (|| -> Result<()> {
            if let Some(branch) = &session.branch {
                std::fs::create_dir_all(
                    session
                        .directory
                        .parent()
                        .context("Missing worktree parent")?,
                )?;
                git::create_worktree(
                    &session.repo,
                    &session.directory,
                    branch,
                    session.base.as_deref().context("Missing base commit")?,
                )?;
            }
            self.runtime.start(&self.store, &session)?;
            Ok(())
        })();
        match result {
            Ok(()) => session.set_state(
                State::Running,
                "Process alive; agent turn status is unknown.",
            ),
            Err(error) => {
                session.set_state(
                    State::Failed,
                    format!("Could not start: {error:#}. Existing files are retained."),
                );
                *registry
                    .sessions
                    .last_mut()
                    .context("Session disappeared")? = session;
                registry.save()?;
                return Err(error);
            }
        }
        *registry
            .sessions
            .last_mut()
            .context("Session disappeared")? = session.clone();
        registry.save()?;
        Ok(session)
    }

    pub fn event(&self, query: &str, state: State, detail: String) -> Result<()> {
        if !matches!(state, State::Waiting | State::Running | State::Review) {
            bail!(
                "Reports may only set waiting, running, or review; process exits are detected automatically"
            );
        }
        let mut registry = self.store.lock()?;
        self.refresh(&mut registry)?;
        let i = registry.find(query)?;
        if !registry.sessions[i].state.active() {
            bail!("This session has no running process");
        }
        registry.sessions[i].set_state(
            state,
            if detail.is_empty() {
                "Explicit status report".into()
            } else {
                detail
            },
        );
        registry.save()
    }

    pub fn stop(&self, query: &str) -> Result<()> {
        let mut registry = self.store.lock()?;
        let i = registry.find(query)?;
        if self
            .runtime
            .panes()?
            .contains_key(&registry.sessions[i].target())
        {
            self.runtime.stop(&registry.sessions[i])?;
        }
        registry.sessions[i].set_state(
            State::Stopped,
            "Stopped by you. Branch and working files retained.",
        );
        registry.save()
    }

    pub fn restart(&self, query: &str) -> Result<Session> {
        let mut registry = self.store.lock()?;
        self.refresh(&mut registry)?;
        let i = registry.find(query)?;
        if registry.sessions[i].state.active() {
            bail!("Session is still running; attach to it intaskstead");
        }
        if !registry.sessions[i].directory.is_dir() {
            bail!("Session directory is missing; no files were recreated");
        }
        executable(&registry.sessions[i].command[0])?;
        if self
            .runtime
            .panes()?
            .contains_key(&registry.sessions[i].target())
        {
            self.runtime.stop(&registry.sessions[i])?;
        }
        registry.sessions[i].set_state(
            State::Starting,
            "Starting a fresh process in the existing worktree",
        );
        registry.sessions[i].exit_code = None;
        registry.save()?;
        if let Err(error) = self.runtime.start(&self.store, &registry.sessions[i]) {
            registry.sessions[i].set_state(State::Failed, error.to_string());
            registry.save()?;
            return Err(error);
        }
        registry.sessions[i].set_state(
            State::Running,
            "Fresh process started; previous agent conversations are not resumed automatically.",
        );
        registry.save()?;
        Ok(registry.sessions[i].clone())
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 48
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("Use 1-48 letters, numbers, hyphens, or underscores for the session name");
    }
    Ok(())
}
