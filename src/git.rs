use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn run(directory: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(["--no-pager", "-C"])
        .arg(directory)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .context("Git is required for worktrees and diffs")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn root(directory: &Path) -> Result<PathBuf> {
    Ok(
        PathBuf::from(run(directory, &["rev-parse", "--show-toplevel"])?.trim_end())
            .canonicalize()?,
    )
}

pub fn head(directory: &Path) -> Result<String> {
    Ok(run(directory, &["rev-parse", "--verify", "HEAD^{commit}"])?
        .trim()
        .into())
}

pub fn create_worktree(repo: &Path, directory: &Path, branch: &str, base: &str) -> Result<()> {
    run(
        repo,
        &[
            "worktree",
            "add",
            "--no-track",
            "-b",
            branch,
            directory
                .to_str()
                .context("Worktree path is not valid UTF-8")?,
            base,
        ],
    )?;
    Ok(())
}

pub fn diff(directory: &Path, base: Option<&str>) -> Result<String> {
    let base = base.context("This session has no Git base commit")?;
    let patch = run(
        directory,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            base,
            "--",
        ],
    )?;
    let untracked = run(directory, &["ls-files", "--others", "--exclude-standard"])?;
    let mut result = if patch.is_empty() {
        "No tracked changes since this session started.\n".into()
    } else {
        patch
    };
    if !untracked.is_empty() {
        result.push_str("\nUntracked files (contents are not included in the patch):\n");
        result.push_str(&untracked);
    }
    Ok(result)
}
