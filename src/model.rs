use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, time::SystemTime};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Starting,
    Running,
    Waiting,
    Review,
    Exited,
    Failed,
    Lost,
    Stopped,
}

impl State {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Waiting | Self::Review
        )
    }

    pub fn priority(self) -> u8 {
        match self {
            Self::Waiting => 0,
            Self::Review => 1,
            Self::Failed | Self::Lost => 2,
            Self::Running | Self::Starting => 3,
            Self::Exited | Self::Stopped => 4,
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Waiting => "needs input",
            Self::Review => "review",
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Lost => "disconnected",
            Self::Stopped => "stopped",
        })
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Agent {
    Codex,
    Claude,
    Shell,
}

impl Agent {
    pub fn command(self) -> Vec<String> {
        match self {
            Self::Codex => vec!["codex".into()],
            Self::Claude => vec!["claude".into()],
            Self::Shell => vec![
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
                "-l".into(),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub repo: PathBuf,
    pub directory: PathBuf,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub command: Vec<String>,
    pub state: State,
    pub detail: String,
    pub created: u64,
    pub updated: u64,
    pub exit_code: Option<i32>,
}

impl Session {
    pub fn set_state(&mut self, state: State, detail: impl Into<String>) {
        self.state = state;
        self.detail = detail.into();
        self.updated = now();
    }

    pub fn target(&self) -> String {
        format!("taskstead-{}", self.id)
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
