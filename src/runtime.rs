use crate::{model::Session, store::Store};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{IsTerminal, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub struct Runtime {
    pub socket: String,
    config: PathBuf,
    socket_path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Pane {
    pub dead: bool,
    pub exit_code: Option<i32>,
}

impl Runtime {
    pub fn new(store: &Store) -> Result<Self> {
        let socket_path = std::env::var_os("TASKSTEAD_SOCKET").map(PathBuf::from);
        if let Some(path) = &socket_path {
            if !path.is_absolute() || !path.parent().is_some_and(Path::is_dir) {
                bail!("TASKSTEAD_SOCKET must be an absolute path inside an existing directory");
            }
            let limit = if cfg!(target_os = "macos") { 103 } else { 107 };
            if path.as_os_str().as_bytes().len() > limit {
                bail!("TASKSTEAD_SOCKET is too long for a Unix socket; use a shorter directory");
            }
        }
        let digest = Sha256::digest(store.root.as_os_str().as_encoded_bytes());
        let socket = format!("taskstead-{:x}", digest);
        let config = store.root.join("tmux.conf");
        if !config.exists() {
            let mut file = tempfile::NamedTempFile::new_in(&store.root)?;
            file.write_all(b"set -g remain-on-exit on\nset -g history-limit 10000\nset -g default-terminal tmux-256color\nset -g status-left ' taskstead '\nset -g status-right 'Ctrl+b d: dashboard'\nset -g status-style 'fg=colour250,bg=colour236'\nset -g allow-rename off\nset -g automatic-rename off\n")?;
            file.as_file().sync_all()?;
            if let Err(error) = file.persist_noclobber(&config)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(error.into());
            }
        }
        Ok(Self {
            socket,
            config,
            socket_path,
        })
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(path) = &self.socket_path {
            command.arg("-S").arg(path);
        } else {
            command.args(["-L", &self.socket]);
        }
        command
            .arg("-f")
            .arg(&self.config)
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR");
        command
    }

    pub fn location(&self) -> String {
        match &self.socket_path {
            Some(path) => format!("tmux -S {}", path.display()),
            None => format!("tmux -L {}", self.socket),
        }
    }

    fn checked(output: Output) -> Result<String> {
        if !output.status.success() {
            bail!("tmux: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn start(&self, store: &Store, session: &Session) -> Result<()> {
        let mut command = self.command();
        command
            .args([
                "new-session",
                "-d",
                "-s",
                &session.target(),
                "-n",
                "agent",
                "-c",
            ])
            .arg(argument(&session.directory))
            .args(
                [
                    "-x",
                    "120",
                    "-y",
                    "36",
                    "-e",
                    &format!("TASKSTEAD_SESSION_ID={}", session.id),
                    "-e",
                    &format!("TASKSTEAD_DATA_DIR={}", store.root.display()),
                    "-e",
                    &format!("TASKSTEAD_BIN={}", std::env::current_exe()?.display()),
                ]
                .map(argument),
            );
        if let Ok(path) = std::env::var("PATH") {
            command.args(["-e", &format!("PATH={path}")].map(argument));
        }
        if let Some(path) = &self.socket_path {
            command.args(["-e", &format!("TASKSTEAD_SOCKET={}", path.display())].map(argument));
        }
        command
            .args(["--", "/usr/bin/env", "--"])
            .args(session.command.iter().map(argument));
        Self::checked(
            command
                .output()
                .context("Install tmux 3.2 or newer to start persistent sessions")?,
        )?;
        Ok(())
    }

    pub fn panes(&self) -> Result<HashMap<String, Pane>> {
        let output = self
            .command()
            .args([
                "list-panes",
                "-a",
                "-F",
                "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_dead}\t#{pane_dead_status}",
            ])
            .output()
            .context("Cannot run tmux; install tmux 3.2 or newer")?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            if error.contains("no server running")
                || error.contains("No such file or directory")
                || error.contains("Connection refused")
            {
                return Ok(HashMap::new());
            }
        }
        let text = Self::checked(output)?;
        let mut panes = HashMap::new();
        for line in text.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() == 5 && fields[1] == "0" && fields[2] == "0" {
                panes.insert(
                    fields[0].into(),
                    Pane {
                        dead: fields[3] == "1",
                        exit_code: fields[4].parse().ok(),
                    },
                );
            }
        }
        Ok(panes)
    }

    pub fn capture(&self, session: &Session) -> Result<String> {
        Self::checked(
            self.command()
                .args([
                    "capture-pane",
                    "-p",
                    "-J",
                    "-t",
                    &format!("{}:0.0", session.target()),
                    "-S",
                    "-150",
                ])
                .output()?,
        )
    }

    pub fn attach(&self, session: &Session) -> Result<()> {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            bail!("Attach needs an interactive terminal; run this command in Ghostty");
        }
        let status = self
            .command()
            .args(["attach-session", "-t", &format!("={}", session.target())])
            .status()?;
        if !status.success() {
            bail!(
                "Could not attach to {}; the session may have ended",
                session.name
            );
        }
        Ok(())
    }

    pub fn stop(&self, session: &Session) -> Result<()> {
        Self::checked(
            self.command()
                .args(["kill-session", "-t", &format!("={}", session.target())])
                .output()?,
        )?;
        Ok(())
    }
}

fn argument(value: impl AsRef<OsStr>) -> OsString {
    let bytes = value.as_ref().as_bytes();
    if let Some(prefix) = bytes.strip_suffix(b";") {
        let mut escaped = prefix.to_vec();
        escaped.extend_from_slice(b"\\;");
        OsString::from_vec(escaped)
    } else {
        value.as_ref().to_os_string()
    }
}

pub fn executable(program: &str) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let valid = |path: &Path| {
        path.is_file()
            && path
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let path = PathBuf::from(program);
        if valid(&path) {
            return Ok(path.canonicalize()?);
        }
    } else if let Some(path) = std::env::var_os("PATH") {
        for root in std::env::split_paths(&path) {
            let candidate = root.join(program);
            if valid(&candidate) {
                return Ok(candidate.canonicalize()?);
            }
        }
    }
    bail!("Cannot find executable {program:?}; install it or choose another agent")
}
