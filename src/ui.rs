use crate::{
    git,
    model::{Agent, Session, State, clean},
    service::{NewSession, Service},
};
use anyhow::{Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, Instant},
};

const ACCENT: Color = Color::Rgb(129, 199, 166);
const MUTED: Color = Color::Rgb(137, 148, 164);
const BORDER: Color = Color::Rgb(66, 77, 91);

enum Modal {
    None,
    Filter,
    New(Form),
    Text {
        title: String,
        body: String,
        scroll: u16,
    },
    Stop(String),
}

struct Form {
    name: String,
    repo: String,
    agent: usize,
    in_place: bool,
    field: usize,
}

struct App {
    sessions: Vec<Session>,
    selection: ListState,
    filter: String,
    modal: Modal,
    notice: String,
    preview: String,
    changed: String,
}

impl App {
    fn visible(&self) -> Vec<&Session> {
        let query = self.filter.to_lowercase();
        self.sessions
            .iter()
            .filter(|s| {
                format!("{} {} {}", s.name, s.agent, s.state)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }

    fn selected(&self) -> Option<Session> {
        self.selection
            .selected()
            .and_then(|i| self.visible().get(i).map(|s| (*s).clone()))
    }

    fn refresh(&mut self, service: &Service) -> Result<()> {
        let selected = self.selected().map(|s| s.id);
        self.sessions = service.list()?;
        let visible = self.visible();
        let next = selected
            .and_then(|id| visible.iter().position(|s| s.id == id))
            .unwrap_or(0);
        self.selection
            .select(if visible.is_empty() { None } else { Some(next) });
        self.refresh_detail(service);
        Ok(())
    }

    fn refresh_detail(&mut self, service: &Service) {
        if let Some(session) = self.selected() {
            self.preview = clean(&service.runtime.capture(&session).unwrap_or_else(|_| {
                "No terminal is available. Your working files are retained.".into()
            }));
            self.changed = git::run(&session.directory, &["status", "--short"])
                .map(|s| {
                    if s.is_empty() {
                        "Working tree clean".into()
                    } else {
                        format!("{} changed or untracked paths", s.lines().count())
                    }
                })
                .unwrap_or_else(|_| "Git status unavailable".into());
        } else {
            self.preview.clear();
            self.changed.clear();
        }
    }

    fn move_selection(&mut self, direction: isize, service: &Service) {
        let count = self.visible().len();
        if count > 0 {
            let current = self.selection.selected().unwrap_or(0);
            self.selection.select(Some(
                (current as isize + direction).rem_euclid(count as isize) as usize,
            ));
            self.refresh_detail(service);
        }
    }
}

pub fn run(service: &Service) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "The dashboard needs a terminal. Run taskstead in Ghostty, or use taskstead list --json"
        );
    }
    let mut app = App {
        sessions: Vec::new(),
        selection: ListState::default(),
        filter: String::new(),
        modal: Modal::None,
        notice: "Sessions stay alive when you close this dashboard.".into(),
        preview: String::new(),
        changed: String::new(),
    };
    app.refresh(service)?;
    let mut terminal = ratatui::try_init()?;
    let result = run_loop(&mut terminal, &mut app, service);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut DefaultTerminal, app: &mut App, service: &Service) -> Result<()> {
    let mut refreshed = Instant::now();
    loop {
        terminal.draw(|frame| render(frame, app))?;
        if event::poll(Duration::from_millis(150))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }
            match handle_key(key, app, terminal, service) {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => app.notice = format!("Error: {}", clean(&format!("{error:#}"))),
            }
        }
        if refreshed.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = app.refresh(service) {
                app.notice = format!("Refresh failed: {}", clean(&error.to_string()));
            }
            refreshed = Instant::now();
        }
    }
    Ok(())
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    terminal: &mut DefaultTerminal,
    service: &Service,
) -> Result<bool> {
    match &mut app.modal {
        Modal::Filter => {
            match key.code {
                KeyCode::Esc => {
                    app.filter.clear();
                    app.modal = Modal::None;
                }
                KeyCode::Enter => app.modal = Modal::None,
                KeyCode::Backspace => {
                    app.filter.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.filter.push(c)
                }
                _ => {}
            }
            app.selection.select(Some(0));
            app.refresh_detail(service);
            return Ok(false);
        }
        Modal::Text { scroll, .. } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => app.modal = Modal::None,
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => *scroll = scroll.saturating_add(15),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(15),
                KeyCode::Home => *scroll = 0,
                _ => {}
            }
            return Ok(false);
        }
        Modal::Stop(id) => {
            let id = id.clone();
            match key.code {
                KeyCode::Char('y') => {
                    service.stop(&id)?;
                    app.modal = Modal::None;
                    app.notice = "Stopped. All branches and files were kept.".into();
                    app.refresh(service)?;
                }
                KeyCode::Esc | KeyCode::Char('n') => app.modal = Modal::None,
                _ => {}
            }
            return Ok(false);
        }
        Modal::New(form) => {
            match key.code {
                KeyCode::Esc => app.modal = Modal::None,
                KeyCode::Tab => form.field = (form.field + 1) % 4,
                KeyCode::BackTab => form.field = (form.field + 3) % 4,
                KeyCode::Enter => {
                    let agent = [Agent::Codex, Agent::Claude, Agent::Shell][form.agent];
                    let session = service.create(NewSession {
                        name: form.name.trim().into(),
                        directory: PathBuf::from(&form.repo),
                        command: agent.command(),
                        in_place: form.in_place,
                    })?;
                    app.modal = Modal::None;
                    app.filter.clear();
                    app.refresh(service)?;
                    app.selection
                        .select(app.sessions.iter().position(|s| s.id == session.id));
                    app.refresh_detail(service);
                    app.notice = format!("{} started. Enter opens its terminal.", session.name);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.field == 2 => {
                    form.agent = (form.agent + 1) % 3
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.field == 3 => {
                    form.in_place = !form.in_place
                }
                KeyCode::Backspace => {
                    if form.field == 0 {
                        form.name.pop();
                    }
                    if form.field == 1 {
                        form.repo.pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if form.field == 0 && form.name.len() < 48 {
                        form.name.push(c);
                    }
                    if form.field == 1 {
                        form.repo.push(c);
                    }
                }
                _ => {}
            }
            return Ok(false);
        }
        Modal::None => {}
    }
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1, service),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1, service),
        KeyCode::Char('/') => app.modal = Modal::Filter,
        KeyCode::Esc => { app.filter.clear(); app.selection.select(Some(0)); app.refresh_detail(service); }
        KeyCode::Char('n') => app.modal = Modal::New(Form {
            name: String::new(), repo: std::env::current_dir()?.to_string_lossy().into_owned(), agent: 0, in_place: false, field: 0,
        }),
        KeyCode::Char('?') => app.modal = Modal::Text {
            title: "A few keys, one workflow".into(), scroll: 0,
            body: "n           New task\nEnter       Open the selected terminal\nCtrl+b d    Detach from a terminal\nj / k       Next / previous task\n/           Filter tasks\nEsc         Clear filter / close dialog\nd           Review changes since task creation\nw           Explicitly mark as needing input\nr           Clear the attention mark\nx           Stop processes (asks for confirmation)\nq           Close the dashboard; sessions keep running\n\nRunning means the process is alive, not that an agent is thinking.\nNeeds input and review are explicit reports, never guesses from screen text.\n\nEach task gets a Git worktree from the current HEAD by default.\nUncommitted files in the original checkout are not copied.\nIn-place tasks share your working files.\n\nClosing Ghostty keeps sessions alive while tmux runs.\nA reboot stops processes; files and task records remain.\ntaskstead restart <task> starts a fresh process in the same directory.\n\nNo automatic approvals, merges, or worktree deletion.".into(),
        },
        KeyCode::Enter => {
            if let Some(session) = app.selected() {
                ratatui::restore();
                let result = service.runtime.attach(&session);
                *terminal = ratatui::try_init()?;
                terminal.clear()?;
                result?;
                app.refresh(service)?;
            }
        }
        KeyCode::Char('d') => {
            if let Some(session) = app.selected() {
                app.modal = Modal::Text { title: format!("{} / changes", session.name), body: clean(&git::diff(&session.directory, session.base.as_deref())?), scroll: 0 };
            }
        }
        KeyCode::Char('x') => { if let Some(session) = app.selected() { app.modal = Modal::Stop(session.id); } }
        KeyCode::Char('w') | KeyCode::Char('r') => {
            if let Some(session) = app.selected() {
                let state = if key.code == KeyCode::Char('w') { State::Waiting } else { State::Running };
                service.event(&session.id, state, "Marked manually in the dashboard".into())?;
                app.refresh(service)?;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn color(state: State) -> Color {
    match state {
        State::Waiting => Color::Yellow,
        State::Failed | State::Lost => Color::LightRed,
        State::Review => Color::LightCyan,
        State::Running | State::Starting => ACCENT,
        State::Exited | State::Stopped => MUTED,
    }
}

fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(title)
}

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 42 || area.height < 14 {
        frame.render_widget(
            Paragraph::new("Taskstead\nEnlarge to at least 42 x 14.\nq closes the dashboard.")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .split(area);
    let attention = app
        .sessions
        .iter()
        .filter(|s| matches!(s.state, State::Waiting | State::Review))
        .count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " TASKSTEAD ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / your tasks, in one place", Style::default().fg(MUTED)),
            Span::raw(format!(
                "     {} tasks  /  {} need you",
                app.sessions.len(),
                attention
            )),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(BORDER)),
        ),
        sections[0],
    );
    let columns = if area.width >= 76 {
        Layout::horizontal([Constraint::Length(30), Constraint::Min(35)])
            .split(sections[1])
            .to_vec()
    } else {
        vec![sections[1]]
    };
    if app.sessions.is_empty() {
        frame.render_widget(Paragraph::new("\n  Make room for focused work.\n\n  n  Start your first task\n\n  Pick a project and an agent.\n  Taskstead keeps its terminal alive\n  and its changes in one worktree.\n\n  ?  See the keys").block(panel(" Tasks ")).wrap(Wrap { trim: false }), columns[0]);
    } else {
        let items: Vec<_> = app
            .visible()
            .iter()
            .map(|s| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        clean(&s.name),
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
                    Line::from(vec![
                        Span::styled(format!("{}", s.state), Style::default().fg(color(s.state))),
                        Span::styled(
                            format!(" / {}", clean(&s.agent)),
                            Style::default().fg(MUTED),
                        ),
                    ]),
                    Line::raw(""),
                ])
            })
            .collect();
        let title = if app.filter.is_empty() {
            " Tasks / attention first ".into()
        } else {
            format!(" Tasks / {} ", clean(&app.filter))
        };
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(title))
                .highlight_symbol(" > ")
                .highlight_style(Style::default().bg(Color::Rgb(35, 47, 55))),
            columns[0],
            &mut app.selection,
        );
    }
    if columns.len() == 2 {
        render_detail(frame, app, columns[1]);
    }
    let notice = if matches!(app.modal, Modal::Filter) {
        format!(" / {}_", app.filter)
    } else {
        format!(" {}", app.notice.replace('\n', " "))
    };
    frame.render_widget(
        Paragraph::new(notice).style(Style::default().fg(if app.notice.starts_with("Error:") {
            Color::LightRed
        } else {
            MUTED
        })),
        sections[2],
    );
    frame.render_widget(
        Paragraph::new(" n new   Enter open   / filter   d diff   ? help   q quit").block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(BORDER)),
        ),
        sections[3],
    );
    render_modal(frame, app);
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(session) = app.selected() else {
        frame.render_widget(Paragraph::new("\n  Select a task to see its terminal.\n\n  Ghostty handles your terminal.\n  Taskstead keeps your tasks together.").style(Style::default().fg(MUTED)).block(panel(" Overview ")), area);
        return;
    };
    let rows = Layout::vertical([Constraint::Length(9), Constraint::Min(3)]).split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{}  /  {}", session.name, session.state),
                Style::default()
                    .fg(color(session.state))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::raw(clean(&session.directory.to_string_lossy())),
            Line::styled(
                session
                    .branch
                    .as_deref()
                    .unwrap_or("In place / shared working files"),
                Style::default().fg(MUTED),
            ),
            Line::raw(""),
            Line::raw(clean(&session.detail)),
            Line::styled(&app.changed, Style::default().fg(MUTED)),
        ])
        .block(panel(" Task "))
        .wrap(Wrap { trim: false }),
        rows[0],
    );
    let height = rows[1].height.saturating_sub(2) as usize;
    let lines: Vec<_> = app.preview.trim_end().lines().collect();
    let body = lines[lines.len().saturating_sub(height)..].join("\n");
    frame.render_widget(
        Paragraph::new(body).block(panel(" Terminal preview / Enter to interact ")),
        rows[1],
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn render_modal(frame: &mut Frame, app: &App) {
    match &app.modal {
        Modal::None | Modal::Filter => {}
        Modal::Text {
            title,
            body,
            scroll,
        } => {
            let area = centered(frame.area(), 110, frame.area().height.saturating_sub(2));
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(body.as_str())
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false })
                    .block(panel(format!(" {title} / arrows scroll / Esc close "))),
                area,
            );
        }
        Modal::Stop(id) => {
            let name = app
                .sessions
                .iter()
                .find(|s| s.id == *id)
                .map(|s| s.name.as_str())
                .unwrap_or(id);
            let area = centered(frame.area(), 60, 8);
            frame.render_widget(Clear, area);
            frame.render_widget(Paragraph::new(format!("\nStop {name} and its processes?\n\nAll branches and working files will be kept.\n\ny stop     n / Esc cancel")).block(panel(" Stop task ")).wrap(Wrap { trim: false }), area);
        }
        Modal::New(form) => {
            let area = centered(frame.area(), 76, 17);
            frame.render_widget(Clear, area);
            let mut lines = vec![Line::raw("")];
            for (i, (label, value)) in [
                ("Task name", form.name.clone()),
                ("Project", form.repo.clone()),
                (
                    "Agent",
                    ["Codex", "Claude Code", "Shell"][form.agent].into(),
                ),
                (
                    "Files",
                    if form.in_place {
                        "In place (shared checkout)"
                    } else {
                        "Isolated worktree from HEAD"
                    }
                    .into(),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let selected = form.field == i;
                lines.push(Line::styled(
                    format!(
                        " {} {label:<10} {value}{}",
                        if selected { ">" } else { " " },
                        if selected && i < 2 { "_" } else { "" }
                    ),
                    Style::default().fg(if selected { ACCENT } else { Color::Reset }),
                ));
                if area.height >= 17 {
                    lines.push(Line::raw(""));
                }
            }
            lines.push(Line::styled(
                " Tab next / arrows change choice",
                Style::default().fg(MUTED),
            ));
            lines.push(Line::raw(" Enter start / Esc cancel"));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                " Worktrees start from committed HEAD.",
                Style::default().fg(MUTED),
            ));
            frame.render_widget(Paragraph::new(lines).block(panel(" New task ")), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn empty_app() -> App {
        App {
            sessions: Vec::new(),
            selection: ListState::default(),
            filter: String::new(),
            modal: Modal::None,
            notice: String::new(),
            preview: String::new(),
            changed: String::new(),
        }
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn empty_dashboard_explains_the_first_action_at_supported_sizes() {
        for (width, height) in [(42, 14), (80, 24), (120, 36)] {
            let output = screen(&mut empty_app(), width, height);
            assert!(output.contains("Start your first task"));
            assert!(output.contains("new"));
        }
    }

    #[test]
    fn new_form_keeps_submit_and_cancel_visible_on_small_terminals() {
        let mut app = empty_app();
        app.modal = Modal::New(Form {
            name: "fix-login".into(),
            repo: "/tmp/project".into(),
            agent: 0,
            in_place: false,
            field: 0,
        });
        for (width, height) in [(42, 14), (80, 24)] {
            let output = screen(&mut app, width, height);
            assert!(output.contains("Enter start / Esc cancel"));
            assert!(output.contains("fix-login"));
            assert!(output.contains("Codex"));
        }
    }

    #[test]
    fn tiny_terminal_has_a_recovery_message() {
        let output = screen(&mut empty_app(), 35, 10);
        assert!(output.contains("Enlarge to at least 42 x 14"));
    }
}
