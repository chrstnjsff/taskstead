use crate::model::Session;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct Registry {
    version: u32,
    sessions: Vec<Session>,
}

pub struct LockedStore {
    root: PathBuf,
    _lock: File,
    pub sessions: Vec<Session>,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        if !root.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)?;
        }
        if fs::symlink_metadata(&root)?.file_type().is_symlink() {
            bail!("The data directory must not be a symbolic link");
        }
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn default_root() -> Result<PathBuf> {
        if let Some(path) =
            std::env::var_os("XDG_STATE_HOME").filter(|p| Path::new(p).is_absolute())
        {
            return Ok(PathBuf::from(path).join("taskstead"));
        }
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("HOME is not set; use --data-dir")?)
                .join(".local/state/taskstead"),
        )
    }

    pub fn lock(&self) -> Result<LockedStore> {
        let path = self.root.join("registry.lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Refusing a symbolic link for the registry lock");
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        lock.lock()?;
        let path = self.root.join("sessions.json");
        let sessions = match File::open(&path) {
            Ok(file) => {
                let registry: Registry = serde_json::from_reader(file).with_context(|| {
                    format!(
                        "Cannot read {}; existing data was preserved",
                        path.display()
                    )
                })?;
                if registry.version != 1 {
                    bail!("Unsupported registry version {}", registry.version);
                }
                for session in &registry.sessions {
                    if uuid::Uuid::parse_str(&session.id).is_err() || session.command.is_empty() {
                        bail!(
                            "Invalid session record in {}; existing data was preserved",
                            path.display()
                        );
                    }
                }
                registry.sessions
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(LockedStore {
            root: self.root.clone(),
            _lock: lock,
            sessions,
        })
    }
}

impl LockedStore {
    pub fn save(&self) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer_pretty(
            &mut file,
            &Registry {
                version: 1,
                sessions: self.sessions.clone(),
            },
        )?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("sessions.json"))?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    pub fn find(&self, query: &str) -> Result<usize> {
        if let Some(i) = self.sessions.iter().position(|s| s.id == query) {
            return Ok(i);
        }
        if let Some(i) = self.sessions.iter().position(|s| s.name == query) {
            return Ok(i);
        }
        let matches: Vec<_> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| !query.is_empty() && s.id.starts_with(query))
            .collect();
        match matches.as_slice() {
            [(i, _)] => Ok(*i),
            [] => bail!("No session matches {query:?}"),
            _ => bail!("More than one session matches {query:?}; use its full ID"),
        }
    }
}
