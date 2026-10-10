use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};
use taskstead::{
    git,
    model::{Session, State},
    service::Service,
};

struct Fixture {
    root: tempfile::TempDir,
    repo: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("project with spaces");
        let data = root.path().join("state");
        fs::create_dir(&repo).unwrap();
        git::run(&repo, &["init", "-b", "main"]).unwrap();
        git::run(&repo, &["config", "user.email", "test@example.invalid"]).unwrap();
        git::run(&repo, &["config", "user.name", "Test"]).unwrap();
        git::run(&repo, &["config", "commit.gpgsign", "false"]).unwrap();
        git::run(&repo, &["config", "core.hooksPath", "/dev/null"]).unwrap();
        fs::write(repo.join("tracked.txt"), "original\n").unwrap();
        git::run(&repo, &["add", "."]).unwrap();
        git::run(&repo, &["commit", "-m", "initial"]).unwrap();
        Self { root, repo, data }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_taskstead"));
        command
            .arg("--data-dir")
            .arg(&self.data)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn start(&self, name: &str, script: &str) -> Session {
        self.run(&[
            "new",
            name,
            "--repo",
            self.repo.to_str().unwrap(),
            "--",
            "/bin/sh",
            "-c",
            script,
        ]);
        self.sessions()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    fn sessions(&self) -> Vec<Session> {
        serde_json::from_slice(&self.run(&["list", "--json"]).stdout).unwrap()
    }

    fn wait_for(&self, name: &str, state: State) -> Session {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let session = self
                .sessions()
                .into_iter()
                .find(|s| s.name == name)
                .unwrap();
            if session.state == state {
                return session;
            }
            assert!(Instant::now() < deadline, "{session:?}");
            thread::sleep(Duration::from_millis(40));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(service) = Service::open(self.data.clone()) {
            let _ = service.runtime.command().arg("kill-server").output();
        }
    }
}

#[test]
fn isolated_process_survives_client_and_reports_exit() {
    let fixture = Fixture::new();
    fs::write(fixture.repo.join("tracked.txt"), "uncommitted user work\n").unwrap();
    let session = fixture.start(
        "isolation",
        "printf 'agent output\\n'; printf 'agent edit\\n' > tracked.txt; sleep 1; exit 7",
    );
    assert_ne!(session.directory, fixture.repo);
    assert_eq!(
        fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(),
        "uncommitted user work\n"
    );
    let finished = fixture.wait_for("isolation", State::Failed);
    assert_eq!(finished.exit_code, Some(7));
    assert_eq!(
        fs::read_to_string(session.directory.join("tracked.txt")).unwrap(),
        "agent edit\n"
    );
    assert!(
        String::from_utf8_lossy(&fixture.run(&["logs", "isolation"]).stdout)
            .contains("agent output")
    );
    assert!(
        git::run(
            &fixture.repo,
            &["branch", "--list", session.branch.as_deref().unwrap()]
        )
        .unwrap()
        .contains("taskstead/isolation")
    );
}

#[test]
fn arguments_are_literal_even_with_shell_metacharacters() {
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("should-not-exist");
    let argument = format!(
        "$(touch '{}'); `touch '{}'` ' spaces \"",
        marker.display(),
        marker.display()
    );
    fixture.run(&[
        "new",
        "literal",
        "--repo",
        fixture.repo.to_str().unwrap(),
        "--",
        "/bin/sh",
        "-c",
        "printf '%s' \"$1\" > literal.txt",
        "fixture",
        &argument,
    ]);
    let session = fixture.wait_for("literal", State::Exited);
    assert_eq!(
        fs::read_to_string(session.directory.join("literal.txt")).unwrap(),
        argument
    );
    assert!(!marker.exists());
}

#[test]
fn tmux_command_separators_are_literal_arguments() {
    let fixture = Fixture::new();
    fixture.run(&[
        "new",
        "separator",
        "--repo",
        fixture.repo.to_str().unwrap(),
        "--",
        "/bin/sh",
        "-c",
        "printf '%s' \"$1\" > literal.txt",
        "fixture",
        "trailing;",
    ]);
    let session = fixture.wait_for("separator", State::Exited);
    assert_eq!(
        fs::read_to_string(session.directory.join("literal.txt")).unwrap(),
        "trailing;"
    );
}

#[test]
fn explicit_events_prioritize_attention_and_reject_dead_sessions() {
    let fixture = Fixture::new();
    fixture.start("first", "sleep 60");
    fixture.start("second", "sleep 60");
    fixture.run(&[
        "event",
        "waiting",
        "--session",
        "first",
        "--detail",
        "Permission required",
    ]);
    let sessions = fixture.sessions();
    assert_eq!(sessions[0].name, "first");
    assert_eq!(sessions[0].state, State::Waiting);
    assert_eq!(sessions[0].detail, "Permission required");
    fixture.run(&["stop", "first", "--yes"]);
    assert!(
        !fixture
            .command()
            .args(["event", "review", "--session", "first"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        fixture
            .sessions()
            .into_iter()
            .find(|s| s.name == "first")
            .unwrap()
            .state,
        State::Stopped
    );
}

#[test]
fn stop_requires_confirmation_and_keeps_worktree() {
    let fixture = Fixture::new();
    let session = fixture.start("keep", "sleep 60");
    let output = fixture.command().args(["stop", "keep"]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fixture.sessions()[0].state, State::Running);
    fs::write(session.directory.join("valuable.txt"), "keep me").unwrap();
    fixture.run(&["stop", "keep", "--yes"]);
    assert_eq!(
        fs::read_to_string(session.directory.join("valuable.txt")).unwrap(),
        "keep me"
    );
    fixture.run(&["restart", "keep"]);
    assert_eq!(fixture.sessions()[0].state, State::Running);
    assert_eq!(
        fs::read_to_string(session.directory.join("valuable.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn runtime_loss_is_honest_and_restart_is_explicit() {
    let fixture = Fixture::new();
    let session = fixture.start("recover", "sleep 60");
    let service = Service::open(fixture.data.clone()).unwrap();
    assert!(
        service
            .runtime
            .command()
            .arg("kill-server")
            .status()
            .unwrap()
            .success()
    );
    let lost = fixture.wait_for("recover", State::Lost);
    assert!(lost.directory.exists());
    assert_eq!(session.directory, lost.directory);
    fixture.run(&["restart", "recover"]);
    assert_eq!(fixture.sessions()[0].state, State::Running);
}

#[test]
fn diff_includes_committed_changes_and_names_untracked_files() {
    let fixture = Fixture::new();
    let session = fixture.start("review", "sleep 60");
    fs::write(
        session.directory.join("tracked.txt"),
        "committed agent change\n",
    )
    .unwrap();
    git::run(&session.directory, &["add", "tracked.txt"]).unwrap();
    git::run(&session.directory, &["commit", "-m", "change"]).unwrap();
    fs::write(session.directory.join("new.txt"), "new file").unwrap();
    let patch = String::from_utf8(fixture.run(&["diff", "review"]).stdout).unwrap();
    assert!(patch.contains("+committed agent change"));
    assert!(patch.contains("Untracked files"));
    assert!(patch.contains("new.txt"));
}

#[test]
fn concurrent_clients_do_not_lose_sessions() {
    let fixture = Fixture::new();
    thread::scope(|scope| {
        for i in 0..6 {
            let fixture = &fixture;
            scope.spawn(move || {
                fixture.run(&[
                    "new",
                    &format!("parallel-{i}"),
                    "--in-place",
                    "--repo",
                    fixture.repo.to_str().unwrap(),
                    "--",
                    "/bin/sleep",
                    "60",
                ]);
            });
        }
    });
    assert_eq!(fixture.sessions().len(), 6);
    assert!(fixture.sessions().iter().all(|s| s.state == State::Running));
}

#[test]
fn invalid_requests_preserve_user_data() {
    let fixture = Fixture::new();
    assert!(
        !fixture
            .command()
            .args(["new", "../escape", "--repo", fixture.repo.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .success()
    );
    fixture.start("unique", "sleep 60");
    assert!(
        !fixture
            .command()
            .args([
                "new",
                "unique",
                "--repo",
                fixture.repo.to_str().unwrap(),
                "--",
                "/bin/sleep",
                "60"
            ])
            .output()
            .unwrap()
            .status
            .success()
    );
    let registry = fixture.data.join("sessions.json");
    let original = fs::read(&registry).unwrap();
    fs::write(&registry, b"broken").unwrap();
    assert!(
        !fixture
            .command()
            .arg("list")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&registry).unwrap(), b"broken");
    fs::write(&registry, original).unwrap();
    assert_eq!(fixture.sessions().len(), 1);
}
