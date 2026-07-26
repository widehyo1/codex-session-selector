use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{
    SessionRow,
    indexer::search::{QueryError, SearchIndex, SearchScope, is_corruption},
    session_date, terminal,
    ui_state::ExecVisibility,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectorAction {
    Quit,
    OpenReplay(PathBuf),
}

pub(crate) struct SelectorApp {
    search_index: SearchIndex,
    total_rows: usize,
    filtered: Vec<SessionRow>,
    list_state: ListState,
    query: String,
    search_scope: SearchScope,
    mode: Mode,
    focus: PaneFocus,
    metadata_scroll: usize,
    message_scroll: u16,
    show_help: bool,
    exec_visibility: ExecVisibility,
    status: Option<String>,
}

impl SelectorApp {
    pub(crate) fn new(search_index: SearchIndex, exec_visibility: ExecVisibility) -> Result<Self> {
        let filtered = search_index
            .search("", SearchScope::All)?
            .into_iter()
            .map(|hit| hit.row)
            .collect::<Vec<_>>();
        let total_rows = filtered.len();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            search_index,
            total_rows,
            filtered,
            list_state,
            query: String::new(),
            search_scope: SearchScope::All,
            mode: Mode::Normal,
            focus: PaneFocus::Sessions,
            metadata_scroll: 0,
            message_scroll: 0,
            show_help: false,
            exec_visibility,
            status: None,
        })
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
        match self.search_index.search(&self.query, self.search_scope) {
            Ok(hits) => {
                self.filtered = hits.into_iter().map(|hit| hit.row).collect();
                self.list_state
                    .select((!self.filtered.is_empty()).then_some(0));
                self.message_scroll = 0;
                self.status = None;
            }
            Err(error) => {
                self.status = Some(format_search_error(&error));
            }
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

    pub(crate) fn exec_visibility(&self) -> ExecVisibility {
        self.exec_visibility
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    pub(crate) fn set_exec_visibility(&mut self, visibility: ExecVisibility) {
        self.exec_visibility = visibility;
    }

    fn toggle_exec_visibility(&mut self) {
        self.exec_visibility.toggle();
        self.status = None;
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

    pub(crate) fn clear_status(&mut self) {
        self.status = None;
    }

    pub(crate) fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<SelectorAction> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(SelectorAction::Quit);
        }

        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => self.show_help = false,
                KeyCode::Char('q') => return Some(SelectorAction::Quit),
                _ => {}
            }
            return None;
        }

        match self.mode {
            Mode::Normal => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Some(SelectorAction::Quit),
                KeyCode::Enter => self.selected_path().map(SelectorAction::OpenReplay),
                KeyCode::Char('e') if key.modifiers.is_empty() => {
                    self.toggle_exec_visibility();
                    None
                }
                KeyCode::Char('/') => {
                    self.mode = Mode::Search;
                    None
                }
                KeyCode::Char('?') => {
                    self.show_help = true;
                    None
                }
                KeyCode::Char('y') => {
                    self.copy_resume_command_to_clipboard();
                    None
                }
                KeyCode::Tab => {
                    self.toggle_focus();
                    None
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    match self.focus {
                        PaneFocus::Sessions => self.move_next(),
                        PaneFocus::Message => self.message_line_down(),
                    }
                    None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    match self.focus {
                        PaneFocus::Sessions => self.move_previous(),
                        PaneFocus::Message => self.message_line_up(),
                    }
                    None
                }
                KeyCode::Char('d') | KeyCode::PageDown => {
                    match self.focus {
                        PaneFocus::Sessions => self.page_down(10),
                        PaneFocus::Message => self.message_page_down(),
                    }
                    None
                }
                KeyCode::Char('u') | KeyCode::PageUp => {
                    match self.focus {
                        PaneFocus::Sessions => self.page_up(10),
                        PaneFocus::Message => self.message_page_up(),
                    }
                    None
                }
                KeyCode::Char('g') | KeyCode::Home => {
                    match self.focus {
                        PaneFocus::Sessions => self.move_first(),
                        PaneFocus::Message => self.message_scroll = 0,
                    }
                    None
                }
                KeyCode::Char('G') | KeyCode::End => {
                    match self.focus {
                        PaneFocus::Sessions => self.move_last(),
                        PaneFocus::Message => self.message_scroll = u16::MAX,
                    }
                    None
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    self.metadata_scroll_left();
                    None
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    self.metadata_scroll_right();
                    None
                }
                KeyCode::Char('0') => {
                    self.metadata_scroll_home();
                    None
                }
                _ => None,
            },
            Mode::Search => {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => self.mode = Mode::Normal,
                    KeyCode::Tab => self.next_search_scope(),
                    KeyCode::Backspace => {
                        self.query.pop();
                        self.refresh_filter();
                    }
                    KeyCode::Char(ch) => {
                        self.query.push(ch);
                        self.refresh_filter();
                    }
                    _ => {}
                }
                None
            }
        }
    }
}

pub(crate) fn run(app: &mut SelectorApp) -> Result<SelectorAction> {
    terminal::with_terminal(|terminal| run_event_loop(terminal, app))
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut SelectorApp,
) -> Result<SelectorAction> {
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if let Some(action) = app.handle_key(key) {
                return Ok(action);
            }
        }
    }
}

fn render(frame: &mut Frame, app: &mut SelectorApp) {
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

fn render_header(frame: &mut Frame, area: Rect, app: &SelectorApp) {
    let selected = app
        .selected_index()
        .map(|index| format!("{}/{}", index + 1, app.filtered.len()))
        .unwrap_or_else(|| "0/0".to_string());

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
        Span::raw(format!(
            " of {}{query} | exec: {}",
            app.total_rows,
            app.exec_visibility.label()
        )),
    ]);

    frame.render_widget(header, area);
}

fn render_list(frame: &mut Frame, area: Rect, app: &mut SelectorApp) {
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

fn render_message(frame: &mut Frame, area: Rect, app: &SelectorApp) {
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

fn render_footer(frame: &mut Frame, area: Rect, app: &SelectorApp) {
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
            Span::raw(" e exec "),
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
            Span::styled(status, Style::default().fg(Color::Yellow)),
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
        Line::raw("  Tab             cycle all/message/cwd/branch/repo/date/exec"),
        Line::raw("  Enter           accept search"),
        Line::raw("  Esc             leave search/help or quit"),
        Line::raw(""),
        Line::raw("Other"),
        Line::raw("  Enter           replay the selected jsonl path"),
        Line::raw("                  return from replay to this selector when replay exits"),
        Line::raw("  e               toggle exec entries for the next replay"),
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
            Self::Date => Self::Exec,
            Self::Exec => Self::All,
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
            Self::Exec => "exec",
        }
    }
}

fn format_search_error(error: &anyhow::Error) -> String {
    if let Some(query_error) = error.downcast_ref::<QueryError>() {
        return format!("search query error: {query_error}");
    }
    if is_corruption(error) {
        return format!("search failed: {error}; run `select-codex-session index --rebuild`");
    }
    format!("search failed: {error}; refresh or rebuild the index")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::IndexOptions,
        indexer::{build_index, store::SessionView},
        test_support::SessionFixture,
    };
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_rows() -> Vec<SessionRow> {
        vec![
            SessionRow {
                path: PathBuf::from("/tmp/a.jsonl"),
                id: None,
                timestamp: Some("2026-05-27T01:00:00Z".to_string()),
                cwd: Some("/repo/alpha".to_string()),
                repository_url: Some("https://git.example/alpha.git".to_string()),
                branch: Some("main".to_string()),
                first_message: "Fix README parser".to_string(),
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

    fn sample_app(visibility: ExecVisibility) -> (SessionFixture, SelectorApp) {
        let fixture = SessionFixture::new();
        fixture.write_named_session("a.jsonl", "Fix README parser", false);
        fixture.write_named_session("b.jsonl", "add selector", false);
        let db = fixture.path("index.sqlite3");
        build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
        .unwrap();
        let index = SearchIndex::open(&db, SessionView::default()).unwrap();
        let app = SelectorApp::new(index, visibility).unwrap();
        (fixture, app)
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
    fn search_scope_cycles_through_exec() {
        let scopes = [
            SearchScope::All,
            SearchScope::FirstMessage,
            SearchScope::Cwd,
            SearchScope::Branch,
            SearchScope::Repository,
            SearchScope::Date,
            SearchScope::Exec,
            SearchScope::All,
        ];

        for pair in scopes.windows(2) {
            assert_eq!(pair[0].next(), pair[1]);
        }
    }

    #[test]
    fn exec_scope_label_is_exec() {
        assert_eq!(SearchScope::All.next(), SearchScope::FirstMessage);
        assert_eq!(SearchScope::Repository.next(), SearchScope::Date);
        assert_eq!(SearchScope::Date.next(), SearchScope::Exec);
        assert_eq!(SearchScope::Exec.label(), "exec");
        assert_eq!(SearchScope::Exec.next(), SearchScope::All);
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

    #[test]
    fn selector_initial_visibility_matches_cli_state() {
        let (_hidden_fixture, hidden) = sample_app(ExecVisibility::Hidden);
        let (_shown_fixture, shown) = sample_app(ExecVisibility::Shown);

        assert_eq!(hidden.exec_visibility(), ExecVisibility::Hidden);
        assert_eq!(shown.exec_visibility(), ExecVisibility::Shown);
        assert_eq!(hidden.filtered.len(), 2);
        assert_eq!(shown.filtered.len(), 2);
    }

    #[test]
    fn selector_e_toggles_only_in_normal_mode() {
        let (_fixture, mut app) = sample_app(ExecVisibility::Hidden);

        assert_eq!(app.handle_key(key(KeyCode::Char('e'))), None);
        assert_eq!(app.exec_visibility(), ExecVisibility::Shown);

        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('e')));
        assert_eq!(app.exec_visibility(), ExecVisibility::Shown);
        assert_eq!(app.query, "e");

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(app.exec_visibility(), ExecVisibility::Shown);
    }

    #[test]
    fn selector_help_is_modal_for_exec_toggle() {
        let (_fixture, mut app) = sample_app(ExecVisibility::Hidden);
        let selected = app.selected_index();
        let focus = app.focus;

        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.show_help);
        for code in [KeyCode::Char('e'), KeyCode::Char('j'), KeyCode::Tab] {
            assert_eq!(app.handle_key(key(code)), None);
        }
        assert_eq!(app.exec_visibility(), ExecVisibility::Hidden);
        assert_eq!(app.selected_index(), selected);
        assert_eq!(app.focus, focus);

        assert_eq!(app.handle_key(key(KeyCode::Esc)), None);
        assert!(!app.show_help);
    }

    #[test]
    fn selector_toggle_preserves_existing_state() {
        let (_fixture, mut app) = sample_app(ExecVisibility::Hidden);
        app.list_state.select(Some(1));
        app.query = "selector".to_string();
        app.search_scope = SearchScope::Branch;
        app.focus = PaneFocus::Message;
        app.metadata_scroll = 8;
        app.message_scroll = 5;
        app.status = Some("transient error".to_string());
        let total_rows = app.total_rows;
        let filtered = app.filtered.clone();

        app.handle_key(key(KeyCode::Char('e')));

        assert_eq!(app.exec_visibility(), ExecVisibility::Shown);
        assert_eq!(app.total_rows, total_rows);
        assert_eq!(app.filtered, filtered);
        assert_eq!(app.selected_index(), Some(1));
        assert_eq!(app.query, "selector");
        assert_eq!(app.search_scope, SearchScope::Branch);
        assert_eq!(app.focus, PaneFocus::Message);
        assert_eq!(app.metadata_scroll, 8);
        assert_eq!(app.message_scroll, 5);
        assert_eq!(app.status, None);
    }

    #[test]
    fn selector_toggle_is_safe_with_empty_search_result() {
        let (_fixture, mut app) = sample_app(ExecVisibility::Hidden);
        app.query = "definitely-not-present".to_string();
        app.refresh_filter();
        app.mode = Mode::Normal;

        app.handle_key(key(KeyCode::Char('e')));

        assert!(app.filtered.is_empty());
        assert_eq!(app.selected_index(), None);
        assert_eq!(app.exec_visibility(), ExecVisibility::Shown);
    }

    #[test]
    fn query_error_preserves_previous_results_and_selection() {
        let (_fixture, mut app) = sample_app(ExecVisibility::Hidden);
        app.query = "fix".to_owned();
        app.refresh_filter();
        let filtered = app.filtered.clone();
        let selected = app.selected_index();
        app.message_scroll = 7;

        app.query = "---".to_owned();
        app.refresh_filter();

        assert_eq!(app.filtered, filtered);
        assert_eq!(app.selected_index(), selected);
        assert_eq!(app.message_scroll, 7);
        assert_eq!(
            app.status.as_deref(),
            Some("search query error: query contains no searchable token")
        );
    }

    #[test]
    fn database_error_preserves_previous_results_and_selection() {
        let (fixture, mut app) = sample_app(ExecVisibility::Hidden);
        let filtered = app.filtered.clone();
        let selected = app.selected_index();
        app.message_scroll = 4;
        let conn = rusqlite::Connection::open(fixture.path("index.sqlite3")).unwrap();
        conn.execute(
            "UPDATE sessions SET first_message = first_message || ' external'",
            [],
        )
        .unwrap();
        drop(conn);

        app.query = "fix".to_owned();
        app.refresh_filter();

        assert_eq!(app.filtered, filtered);
        assert_eq!(app.selected_index(), selected);
        assert_eq!(app.message_scroll, 4);
        assert!(
            app.status
                .as_deref()
                .unwrap()
                .starts_with("search failed: FTS index is dirty")
        );
    }
}
