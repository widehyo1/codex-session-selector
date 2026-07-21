use std::{env, io, path::PathBuf, process::Command, time::Duration};

use anyhow::{Result, bail};
use codex_session_selector::{SessionRow, load_sessions, session_date};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;
    if args.refresh {
        refresh_database(&args)?;
    }

    let rows = load_sessions(&args.db)?;
    if rows.is_empty() {
        bail!("no sessions found in {}", args.db.display());
    }

    let mut app = App::new(rows);

    if args.print_path {
        let mut terminal = ratatui::init();
        let action = run_app(&mut terminal, &mut app);
        restore_terminal();

        let AppAction::Replay(path) = action? else {
            return Ok(());
        };
        println!("{}", path.display());
        return Ok(());
    }

    loop {
        let mut terminal = ratatui::init();
        let action = run_app(&mut terminal, &mut app);
        restore_terminal();

        match action? {
            AppAction::Quit => return Ok(()),
            AppAction::Replay(path) => {
                let mut command = Command::new(&args.replay_command);
                if args.include_exec {
                    command.arg("--include-exec");
                }
                let status = command.arg(path).status()?;
                app.status = if status.success() {
                    None
                } else {
                    Some(format!(
                        "{} exited with status {}",
                        args.replay_command,
                        status.code().unwrap_or(1)
                    ))
                };
            }
        }
    }
}

struct Args {
    db: PathBuf,
    replay_command: String,
    record_command: String,
    refresh: bool,
    print_path: bool,
    include_exec: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut parsed = Self {
            db: home_dir()?.join("codex-session-info.sqlite3"),
            replay_command: "codex-replay-tui".to_string(),
            record_command: default_record_command(),
            refresh: true,
            print_path: false,
            include_exec: false,
        };

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--db" => {
                    let Some(value) = args.next() else {
                        bail!("{arg} requires a path");
                    };
                    parsed.db = expand_home(&value)?;
                }
                "--replay-command" => {
                    let Some(value) = args.next() else {
                        bail!("{arg} requires a command");
                    };
                    parsed.replay_command = value;
                }
                "--record-command" => {
                    let Some(value) = args.next() else {
                        bail!("{arg} requires a command");
                    };
                    parsed.record_command = value;
                }
                "--no-refresh" => parsed.refresh = false,
                "--print-path" => parsed.print_path = true,
                "--include-exec" => parsed.include_exec = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                "-V" | "--version" => {
                    println!("select-codex-session {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(parsed)
    }
}

fn refresh_database(args: &Args) -> Result<()> {
    let status = Command::new(&args.record_command)
        .args(record_command_args(&args.db, args.include_exec))
        .status()?;

    if !status.success() {
        bail!(
            "{} failed with status {}",
            args.record_command,
            status.code().unwrap_or(1)
        );
    }

    Ok(())
}

fn record_command_args(db: &std::path::Path, include_exec: bool) -> Vec<String> {
    let mut args = vec!["--output".to_string(), db.to_string_lossy().to_string()];
    if include_exec {
        args.push("--include-exec".to_string());
    }
    args
}

fn default_record_command() -> String {
    env::current_exe()
        .ok()
        .as_deref()
        .and_then(default_record_command_from_exe)
        .unwrap_or_else(|| "record-codex-session-info".to_string())
}

fn default_record_command_from_exe(exe: &std::path::Path) -> Option<String> {
    let candidate = exe.parent()?.join("record-codex-session-info");
    candidate
        .exists()
        .then(|| candidate.to_string_lossy().to_string())
}

fn restore_terminal() {
    ratatui::restore();
    let _ = execute!(io::stdout(), cursor::Show);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneFocus {
    Sessions,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchScope {
    All,
    FirstMessage,
    Cwd,
    Branch,
    Repository,
    Date,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppAction {
    Quit,
    Replay(PathBuf),
}

struct App {
    rows: Vec<SessionRow>,
    filtered: Vec<SessionRow>,
    list_state: ListState,
    query: String,
    search_scope: SearchScope,
    mode: Mode,
    focus: PaneFocus,
    metadata_scroll: usize,
    message_scroll: u16,
    show_help: bool,
    status: Option<String>,
}

impl App {
    fn new(rows: Vec<SessionRow>) -> Self {
        let filtered = rows.clone();
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            rows,
            filtered,
            list_state,
            query: String::new(),
            search_scope: SearchScope::All,
            mode: Mode::Normal,
            focus: PaneFocus::Sessions,
            metadata_scroll: 0,
            message_scroll: 0,
            show_help: false,
            status: None,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn selected_row(&self) -> Option<&SessionRow> {
        self.selected_index()
            .and_then(|index| self.filtered.get(index))
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selected_row().map(|row| row.path.clone())
    }

    fn selected_resume_command(&self) -> Option<String> {
        self.selected_row().and_then(resume_command)
    }

    fn refresh_filter(&mut self) {
        self.filtered = filter_sessions_by_scope(&self.rows, &self.query, self.search_scope);
        self.message_scroll = 0;
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            PaneFocus::Sessions => PaneFocus::Message,
            PaneFocus::Message => PaneFocus::Sessions,
        };
    }

    fn next_search_scope(&mut self) {
        self.search_scope = self.search_scope.next();
        self.refresh_filter();
    }

    fn metadata_scroll_right(&mut self) {
        self.metadata_scroll = self.metadata_scroll.saturating_add(4);
    }

    fn metadata_scroll_left(&mut self) {
        self.metadata_scroll = self.metadata_scroll.saturating_sub(4);
    }

    fn metadata_scroll_home(&mut self) {
        self.metadata_scroll = 0;
    }

    fn message_line_down(&mut self) {
        self.message_scroll = self.message_scroll.saturating_add(1);
    }

    fn message_line_up(&mut self) {
        self.message_scroll = self.message_scroll.saturating_sub(1);
    }

    fn message_page_down(&mut self) {
        self.message_scroll = self.message_scroll.saturating_add(10);
    }

    fn message_page_up(&mut self) {
        self.message_scroll = self.message_scroll.saturating_sub(10);
    }

    fn copy_resume_command_to_clipboard(&mut self) {
        let Some(command) = self.selected_resume_command() else {
            self.status = Some("selected session has no id".to_string());
            return;
        };

        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(command.clone()))
        {
            Ok(()) => {
                self.status = Some(format!("copied: {command}"));
            }
            Err(err) => {
                self.status = Some(format!("clipboard copy failed: {err}"));
            }
        }
    }

    fn move_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let next = match self.selected_index() {
            Some(index) if index + 1 < self.filtered.len() => index + 1,
            _ => 0,
        };
        self.list_state.select(Some(next));
        self.message_scroll = 0;
    }

    fn move_previous(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let previous = match self.selected_index() {
            Some(0) | None => self.filtered.len() - 1,
            Some(index) => index - 1,
        };
        self.list_state.select(Some(previous));
        self.message_scroll = 0;
    }

    fn move_first(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
            self.message_scroll = 0;
        }
    }

    fn move_last(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(self.filtered.len() - 1));
            self.message_scroll = 0;
        }
    }

    fn page_down(&mut self, amount: usize) {
        for _ in 0..amount.max(1) {
            self.move_next();
        }
    }

    fn page_up(&mut self, amount: usize) {
        for _ in 0..amount.max(1) {
            self.move_previous();
        }
    }
}

fn run_app(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<AppAction> {
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match app.mode {
                    Mode::Normal => match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(AppAction::Quit);
                        }
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(AppAction::Quit),
                        KeyCode::Enter => {
                            if let Some(path) = app.selected_path() {
                                return Ok(AppAction::Replay(path));
                            }
                        }
                        KeyCode::Char('/') => app.mode = Mode::Search,
                        KeyCode::Char('?') => app.show_help = !app.show_help,
                        KeyCode::Char('y') => app.copy_resume_command_to_clipboard(),
                        KeyCode::Tab => app.toggle_focus(),
                        KeyCode::Char('j') | KeyCode::Down => match app.focus {
                            PaneFocus::Sessions => app.move_next(),
                            PaneFocus::Message => app.message_line_down(),
                        },
                        KeyCode::Char('k') | KeyCode::Up => match app.focus {
                            PaneFocus::Sessions => app.move_previous(),
                            PaneFocus::Message => app.message_line_up(),
                        },
                        KeyCode::Char('d') | KeyCode::PageDown => match app.focus {
                            PaneFocus::Sessions => app.page_down(10),
                            PaneFocus::Message => app.message_page_down(),
                        },
                        KeyCode::Char('u') | KeyCode::PageUp => match app.focus {
                            PaneFocus::Sessions => app.page_up(10),
                            PaneFocus::Message => app.message_page_up(),
                        },
                        KeyCode::Char('g') | KeyCode::Home => match app.focus {
                            PaneFocus::Sessions => app.move_first(),
                            PaneFocus::Message => app.message_scroll = 0,
                        },
                        KeyCode::Char('G') | KeyCode::End => match app.focus {
                            PaneFocus::Sessions => app.move_last(),
                            PaneFocus::Message => app.message_scroll = u16::MAX,
                        },
                        KeyCode::Char('h') | KeyCode::Left => app.metadata_scroll_left(),
                        KeyCode::Char('l') | KeyCode::Right => app.metadata_scroll_right(),
                        KeyCode::Char('0') => app.metadata_scroll_home(),
                        _ => {}
                    },
                    Mode::Search => match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(AppAction::Quit);
                        }
                        KeyCode::Esc => app.mode = Mode::Normal,
                        KeyCode::Enter => app.mode = Mode::Normal,
                        KeyCode::Tab => app.next_search_scope(),
                        KeyCode::Backspace => {
                            app.query.pop();
                            app.refresh_filter();
                        }
                        KeyCode::Char(ch) => {
                            app.query.push(ch);
                            app.refresh_filter();
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn render(frame: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, root[0], app);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(root[1]);

    render_list(frame, body[0], app);
    render_message(frame, body[1], app);
    render_footer(frame, root[2], app);

    if app.show_help {
        render_help(frame);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let selected = app
        .selected_index()
        .map(|index| format!("{}/{}", index + 1, app.filtered.len()))
        .unwrap_or_else(|| format!("0/{}", app.filtered.len()));

    let query = if app.query.is_empty() {
        String::new()
    } else {
        format!(" | /{}", app.query)
    };

    let header = Line::from(vec![
        Span::styled(
            " Codex Sessions ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(selected, Style::default().fg(Color::Cyan)),
        Span::raw(format!(" of {}{query}", app.rows.len())),
    ]);

    frame.render_widget(header, area);
}

fn render_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .map(|row| ListItem::new(metadata_line(row, app.metadata_scroll)))
        .collect();

    let title = if app.mode == Mode::Search {
        " Sessions [search] "
    } else if app.focus == PaneFocus::Sessions {
        " Sessions [focus] "
    } else {
        " Sessions "
    };

    let border_style = if app.mode == Mode::Search {
        Style::default().fg(Color::Yellow)
    } else if app.focus == PaneFocus::Sessions {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items)
        .block(
            Block::new()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_message(frame: &mut Frame, area: Rect, app: &App) {
    let Some(row) = app.selected_row() else {
        let empty = Paragraph::new("No matching sessions")
            .block(Block::new().title(" First Message ").borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    };

    let title = if app.focus == PaneFocus::Message {
        format!(" First Message [focus]: {} ", session_date(row))
    } else {
        format!(" First Message: {} ", session_date(row))
    };
    let border_style = if app.focus == PaneFocus::Message {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let text = Text::from(
        row.first_message
            .lines()
            .map(|line| Line::raw(line.to_string()))
            .collect::<Vec<_>>(),
    );

    let paragraph = Paragraph::new(text)
        .block(
            Block::new()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.message_scroll, 0));

    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let status = app
        .status
        .as_deref()
        .map(|status| format!(" | {status}"))
        .unwrap_or_default();

    let footer = match app.mode {
        Mode::Normal => Line::from(vec![
            Span::raw(" focus: "),
            Span::styled(app.focus.label(), Style::default().fg(Color::Cyan)),
            Span::raw(" | "),
            Span::raw(" j/k ↑/↓ move "),
            Span::raw(" d/u page "),
            Span::raw(" g/G first/last "),
            Span::raw(" h/l ←/→ x-scroll "),
            Span::raw(" Tab focus "),
            Span::raw(" / search "),
            Span::raw(" Enter replay "),
            Span::raw(" y copy-resume "),
            Span::raw(" q quit "),
            Span::raw(" Ctrl-C quit "),
            Span::raw(" ? help "),
            Span::styled(status, Style::default().fg(Color::Yellow)),
        ]),
        Mode::Search => Line::from(vec![
            Span::raw(" search: "),
            Span::styled(app.search_scope.label(), Style::default().fg(Color::Cyan)),
            Span::raw(" /"),
            Span::styled(app.query.clone(), Style::default().fg(Color::Yellow)),
            Span::raw("  Tab scope  Enter accept  Esc cancel  Ctrl-C quit  Backspace delete "),
        ]),
    }
    .gray();

    frame.render_widget(footer, area);
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(76, 70, frame.area());
    let help = Paragraph::new(vec![
        Line::styled(
            "Codex Session Selector",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("Left pane shows date, cwd, branch, and repository URL."),
        Line::raw("Right pane shows the selected session's first user message."),
        Line::raw(""),
        Line::raw("Focus"),
        Line::raw("  Tab             switch focus between sessions/message"),
        Line::raw(""),
        Line::raw("Movement"),
        Line::raw("  j / Down        next session"),
        Line::raw("  k / Up          previous session"),
        Line::raw("  d / PageDown    page down"),
        Line::raw("  u / PageUp      page up"),
        Line::raw("  g / Home        first session"),
        Line::raw("  G / End         last session"),
        Line::raw("  h / Left        horizontal scroll left in sessions pane"),
        Line::raw("  l / Right       horizontal scroll right in sessions pane"),
        Line::raw("  0               reset horizontal scroll"),
        Line::raw(""),
        Line::raw("Search"),
        Line::raw("  /               interactive search"),
        Line::raw("  Tab             cycle all/message/cwd/branch/repo/date"),
        Line::raw("  Enter           accept search"),
        Line::raw("  Esc             leave search/help or quit"),
        Line::raw(""),
        Line::raw("Other"),
        Line::raw("  Enter           run codex-replay-tui with selected jsonl path"),
        Line::raw("                  return from replay to this selector when replay exits"),
        Line::raw("  y               copy `codex resume <session-id>` to clipboard"),
        Line::raw("  ?               toggle help"),
        Line::raw("  q / Ctrl-C      quit"),
    ])
    .block(Block::new().title(" Help ").borders(Borders::ALL))
    .wrap(Wrap { trim: false });

    frame.render_widget(Clear, area);
    frame.render_widget(help, area);
}

fn metadata_line(row: &SessionRow, scroll: usize) -> Line<'static> {
    let segments = metadata_segments(row);
    scrolled_segments(&segments, scroll)
}

fn metadata_segments(row: &SessionRow) -> Vec<(String, Style)> {
    vec![
        (
            dash_empty(&session_date(row)).to_string(),
            Style::default().fg(Color::Cyan),
        ),
        (" | ".to_string(), Style::default().fg(Color::DarkGray)),
        (
            dash_empty(row.cwd.as_deref().unwrap_or_default()).to_string(),
            Style::default().fg(Color::Green),
        ),
        (" | ".to_string(), Style::default().fg(Color::DarkGray)),
        (
            dash_empty(row.branch.as_deref().unwrap_or_default()).to_string(),
            Style::default().fg(Color::Yellow),
        ),
        (" | ".to_string(), Style::default().fg(Color::DarkGray)),
        (
            dash_empty(row.repository_url.as_deref().unwrap_or_default()).to_string(),
            Style::default().fg(Color::Magenta),
        ),
    ]
}

#[cfg(test)]
fn metadata_plain(row: &SessionRow) -> String {
    metadata_segments(row)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("")
}

fn scrolled_segments(segments: &[(String, Style)], scroll: usize) -> Line<'static> {
    let mut remaining = scroll;
    let mut spans = Vec::new();

    for (text, style) in segments {
        let char_count = text.chars().count();
        if remaining >= char_count {
            remaining -= char_count;
            continue;
        }

        let visible = text.chars().skip(remaining).collect::<String>();
        remaining = 0;
        if !visible.is_empty() {
            spans.push(Span::styled(visible, *style));
        }
    }

    Line::from(spans)
}

fn filter_sessions_by_scope(
    rows: &[SessionRow],
    query: &str,
    search_scope: SearchScope,
) -> Vec<SessionRow> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return rows.to_vec();
    }

    rows.iter()
        .filter(|row| {
            let haystack = search_scope.text(row).to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .cloned()
        .collect()
}

fn resume_command(row: &SessionRow) -> Option<String> {
    row.id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(|id| format!("codex resume {id}"))
}

fn dash_empty(value: &str) -> &str {
    if value.trim().is_empty() { "-" } else { value }
}

impl PaneFocus {
    fn label(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Message => "message",
        }
    }
}

impl SearchScope {
    fn next(self) -> Self {
        match self {
            Self::All => Self::FirstMessage,
            Self::FirstMessage => Self::Cwd,
            Self::Cwd => Self::Branch,
            Self::Branch => Self::Repository,
            Self::Repository => Self::Date,
            Self::Date => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::FirstMessage => "message",
            Self::Cwd => "cwd",
            Self::Branch => "branch",
            Self::Repository => "repo",
            Self::Date => "date",
        }
    }

    fn text(self, row: &SessionRow) -> String {
        match self {
            Self::All => [
                row.first_message.as_str(),
                row.cwd.as_deref().unwrap_or_default(),
                row.repository_url.as_deref().unwrap_or_default(),
                row.branch.as_deref().unwrap_or_default(),
                row.timestamp.as_deref().unwrap_or_default(),
                &session_date(row),
            ]
            .join("\n"),
            Self::FirstMessage => row.first_message.clone(),
            Self::Cwd => row.cwd.clone().unwrap_or_default(),
            Self::Branch => row.branch.clone().unwrap_or_default(),
            Self::Repository => row.repository_url.clone().unwrap_or_default(),
            Self::Date => session_date(row),
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn print_help() {
    println!(
        "\
select-codex-session {version}

Open a TUI for choosing a local Codex session and replaying its JSONL timeline.

Usage:
  select-codex-session [OPTIONS]

Options:
      --db PATH                  SQLite index path
                                 default: ~/codex-session-info.sqlite3
      --record-command COMMAND   Recorder binary used during refresh
                                 default: sibling record-codex-session-info, then PATH
      --replay-command COMMAND   Replay binary used after selection
                                 default: codex-replay-tui
      --no-refresh               Do not rebuild the SQLite index before opening
      --include-exec             Index and show command execution records
                                 passed to recorder during refresh and replay after selection
      --print-path               Print the selected JSONL path instead of replaying
  -h, --help                     Show this help
  -V, --version                  Show version

Keys:
  Enter                          replay selected session, then return here
  /                              interactive search
  Tab                            switch pane focus; while searching, cycle search scope
  h/l or Left/Right              horizontal scroll in sessions pane
  y                              copy `codex resume <session-id>` to clipboard
  q, Esc, Ctrl-C                 quit
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn expand_home(value: &str) -> Result<PathBuf> {
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn sample_rows() -> Vec<SessionRow> {
        vec![
            SessionRow {
                path: PathBuf::from("/tmp/a.jsonl"),
                id: None,
                timestamp: Some("2026-05-27T01:00:00Z".to_string()),
                cwd: Some("/repo/alpha".to_string()),
                repository_url: Some("https://git.example/alpha.git".to_string()),
                branch: Some("main".to_string()),
                first_message: "fix docker compose".to_string(),
                is_subsession: false,
            },
            SessionRow {
                path: PathBuf::from("/tmp/b.jsonl"),
                id: None,
                timestamp: Some("2026-05-28T01:00:00Z".to_string()),
                cwd: Some("/repo/beta".to_string()),
                repository_url: Some("https://git.example/beta.git".to_string()),
                branch: Some("feature/search".to_string()),
                first_message: "add selector".to_string(),
                is_subsession: false,
            },
        ]
    }

    #[test]
    fn record_command_args_write_to_selected_db() {
        assert_eq!(
            record_command_args(Path::new("/tmp/session.sqlite3"), false),
            vec!["--output".to_string(), "/tmp/session.sqlite3".to_string()]
        );
        assert_eq!(
            record_command_args(Path::new("/tmp/session.sqlite3"), true),
            vec![
                "--output".to_string(),
                "/tmp/session.sqlite3".to_string(),
                "--include-exec".to_string()
            ]
        );
    }

    #[test]
    fn default_record_command_prefers_sibling_binary() {
        let dir = std::env::temp_dir().join(format!(
            "codex-session-selector-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("select-codex-session");
        let recorder = dir.join("record-codex-session-info");
        std::fs::write(&recorder, "").unwrap();

        assert_eq!(
            default_record_command_from_exe(&exe),
            Some(recorder.to_string_lossy().to_string())
        );

        let _ = std::fs::remove_file(recorder);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn args_refresh_by_default_and_can_disable_it() {
        let args = Args::parse(std::iter::empty()).unwrap();
        assert!(args.refresh);
        assert!(args.record_command.ends_with("record-codex-session-info"));
        assert!(!args.include_exec);

        let args = Args::parse(["--no-refresh".to_string()].into_iter()).unwrap();
        assert!(!args.refresh);

        let args = Args::parse(["--include-exec".to_string()].into_iter()).unwrap();
        assert!(args.include_exec);
    }

    #[test]
    fn metadata_plain_uses_requested_column_order() {
        let rows = sample_rows();

        assert_eq!(
            metadata_plain(&rows[1]),
            "2026-05-28 | /repo/beta | feature/search | https://git.example/beta.git"
        );
    }

    #[test]
    fn search_scope_filters_only_selected_field() {
        let rows = sample_rows();

        assert_eq!(
            filter_sessions_by_scope(&rows, "docker", SearchScope::FirstMessage)[0].path,
            PathBuf::from("/tmp/a.jsonl")
        );
        assert!(filter_sessions_by_scope(&rows, "docker", SearchScope::Cwd).is_empty());
        assert_eq!(
            filter_sessions_by_scope(&rows, "beta", SearchScope::Cwd)[0].path,
            PathBuf::from("/tmp/b.jsonl")
        );
        assert_eq!(
            filter_sessions_by_scope(&rows, "feature", SearchScope::Branch)[0].path,
            PathBuf::from("/tmp/b.jsonl")
        );
        assert_eq!(
            filter_sessions_by_scope(&rows, "alpha.git", SearchScope::Repository)[0].path,
            PathBuf::from("/tmp/a.jsonl")
        );
        assert_eq!(
            filter_sessions_by_scope(&rows, "2026-05-28", SearchScope::Date)[0].path,
            PathBuf::from("/tmp/b.jsonl")
        );
    }

    #[test]
    fn search_scope_cycles_through_all_fields() {
        let scopes = [
            SearchScope::All,
            SearchScope::FirstMessage,
            SearchScope::Cwd,
            SearchScope::Branch,
            SearchScope::Repository,
            SearchScope::Date,
            SearchScope::All,
        ];

        for pair in scopes.windows(2) {
            assert_eq!(pair[0].next(), pair[1]);
        }
    }

    #[test]
    fn scrolled_segments_preserve_text_after_horizontal_offset() {
        let rows = sample_rows();
        let line = scrolled_segments(&metadata_segments(&rows[1]), 13);
        let text = line
            .spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>()
            .join("");

        assert!(text.starts_with("/repo/beta"));
    }

    #[test]
    fn resume_command_uses_session_id() {
        let mut rows = sample_rows();
        rows[0].id = Some("019e6ce6-08f8-77e0-ba64-f303967970e0".to_string());

        assert_eq!(
            resume_command(&rows[0]).as_deref(),
            Some("codex resume 019e6ce6-08f8-77e0-ba64-f303967970e0")
        );

        rows[0].id = None;
        assert_eq!(resume_command(&rows[0]), None);
    }
}
