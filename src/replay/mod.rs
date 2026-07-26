use std::{collections::HashMap, io::Read, path::Path, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;

use crate::{cli::ReplayOptions, terminal};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum PayloadEvent {
    #[serde(rename = "user_message")]
    UserMessage {
        message: String,
        #[serde(default)]
        phase: Option<String>,
    },

    #[serde(rename = "agent_message")]
    AgentMessage {
        message: String,
        #[serde(default)]
        phase: Option<String>,
    },

    #[serde(rename = "exec_command_end")]
    ExecCommandEnd {
        #[serde(default)]
        parsed_cmd: Vec<ParsedCmd>,
        #[serde(default)]
        aggregated_output: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct RawRecord {
    #[serde(default)]
    r#type: Option<String>,

    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParsedCmd {
    #[serde(default)]
    r#type: Option<String>,

    #[serde(default)]
    cmd: String,

    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone)]
enum NormalizedEvent {
    Payload(PayloadEvent),
    ExecToolCall {
        call_id: Option<String>,
        kind: String,
        name: String,
        input: String,
    },
    ExecToolOutput {
        call_id: Option<String>,
        output: String,
    },
}

#[derive(Debug, Clone)]
enum EntryKind {
    User,
    Agent,
    Exec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneFocus {
    Timeline,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fullscreen {
    None,
    Timeline,
    Detail,
}

#[derive(Debug, Clone)]
struct Entry {
    index: usize,
    kind: EntryKind,
    title: String,
    detail: String,
}

struct ReplayApp {
    entries: Vec<Entry>,
    list_state: ListState,
    detail_scroll: u16,
    focus: PaneFocus,
    fullscreen: Fullscreen,
    show_help: bool,
    status: Option<String>,
}

impl ReplayApp {
    fn new(entries: Vec<Entry>) -> Self {
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            entries,
            list_state,
            detail_scroll: 0,
            focus: PaneFocus::Timeline,
            fullscreen: Fullscreen::None,
            show_help: false,
            status: None,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.selected_index().and_then(|i| self.entries.get(i))
    }

    fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let next = match self.selected_index() {
            Some(i) if i + 1 < self.entries.len() => i + 1,
            _ => 0,
        };

        self.list_state.select(Some(next));
        self.detail_scroll = 0;
        self.status = None;
    }

    fn previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let previous = match self.selected_index() {
            Some(0) | None => self.entries.len() - 1,
            Some(i) => i - 1,
        };

        self.list_state.select(Some(previous));
        self.detail_scroll = 0;
        self.status = None;
    }

    fn detail_page_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(10);
        self.status = None;
    }

    fn detail_page_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(10);
        self.status = None;
    }

    fn detail_line_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
        self.status = None;
    }

    fn detail_line_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
        self.status = None;
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            PaneFocus::Timeline => PaneFocus::Detail,
            PaneFocus::Detail => PaneFocus::Timeline,
        };
        self.status = None;
    }

    fn toggle_timeline_fullscreen(&mut self) {
        self.fullscreen = match self.fullscreen {
            Fullscreen::Timeline => Fullscreen::None,
            _ => Fullscreen::Timeline,
        };
        self.status = None;
    }

    fn toggle_detail_fullscreen(&mut self) {
        self.fullscreen = match self.fullscreen {
            Fullscreen::Detail => Fullscreen::None,
            _ => Fullscreen::Detail,
        };
        self.status = None;
    }

    fn exit_fullscreen(&mut self) {
        self.fullscreen = Fullscreen::None;
        self.status = None;
    }

    fn selected_detail_string(&self) -> Option<String> {
        let entry = self.selected_entry()?;

        Some(format!("{}\n\n{}", entry.title, entry.detail))
    }

    fn copy_detail_to_clipboard(&mut self) {
        let Some(text) = self.selected_detail_string() else {
            self.status = Some("nothing to copy".to_string());
            return;
        };

        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            Ok(()) => {
                self.status = Some("copied detail pane to clipboard".to_string());
            }
            Err(err) => {
                self.status = Some(format!("clipboard copy failed: {err}"));
            }
        }
    }
}

pub(crate) fn run(options: &ReplayOptions) -> Result<()> {
    let input = read_input(options.input.as_deref())?;
    let entries = load_entries_from_str(&input, options.include_exec)?;

    terminal::with_terminal(|terminal| run_event_loop(terminal, ReplayApp::new(entries)))
}

fn read_input(input: Option<&Path>) -> Result<String> {
    match input {
        Some(path) if path != Path::new("-") => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display())),
        Some(_) | None => {
            let mut stdin = std::io::stdin().lock();
            let input = read_input_from_reader(&mut stdin)?;

            if input.trim().is_empty() {
                anyhow::bail!(
                    "usage: select-codex-session replay <events.json|events.jsonl>\n       jq -c . a.json | select-codex-session replay"
                );
            }

            Ok(input)
        }
    }
}

fn read_input_from_reader(reader: &mut impl Read) -> Result<String> {
    let mut input = String::new();
    reader
        .read_to_string(&mut input)
        .context("failed to read stdin")?;
    Ok(input)
}

fn run_event_loop(terminal: &mut ratatui::DefaultTerminal, mut app: ReplayApp) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('q') => break,

                    KeyCode::Esc => {
                        if app.fullscreen != Fullscreen::None {
                            app.exit_fullscreen();
                        } else if app.show_help {
                            app.show_help = false;
                        } else {
                            break;
                        }
                    }

                    KeyCode::Tab => app.toggle_focus(),

                    KeyCode::Char('1') => app.toggle_timeline_fullscreen(),
                    KeyCode::Char('2') => app.toggle_detail_fullscreen(),
                    KeyCode::Char('f') => match app.focus {
                        PaneFocus::Timeline => app.toggle_timeline_fullscreen(),
                        PaneFocus::Detail => app.toggle_detail_fullscreen(),
                    },

                    KeyCode::Char('y') => {
                        if app.focus == PaneFocus::Detail || app.fullscreen == Fullscreen::Detail {
                            app.copy_detail_to_clipboard();
                        }
                    }

                    KeyCode::Char('j') | KeyCode::Down => match app.focus {
                        PaneFocus::Timeline => app.next(),
                        PaneFocus::Detail => app.detail_line_down(),
                    },

                    KeyCode::Char('k') | KeyCode::Up => match app.focus {
                        PaneFocus::Timeline => app.previous(),
                        PaneFocus::Detail => app.detail_line_up(),
                    },

                    KeyCode::Char('d') | KeyCode::PageDown => match app.focus {
                        PaneFocus::Timeline => app.next(),
                        PaneFocus::Detail => app.detail_page_down(),
                    },

                    KeyCode::Char('u') | KeyCode::PageUp => match app.focus {
                        PaneFocus::Timeline => app.previous(),
                        PaneFocus::Detail => app.detail_page_up(),
                    },

                    KeyCode::Char('g') | KeyCode::Home => {
                        match app.focus {
                            PaneFocus::Timeline => {
                                app.list_state.select(Some(0));
                                app.detail_scroll = 0;
                            }
                            PaneFocus::Detail => {
                                app.detail_scroll = 0;
                            }
                        }
                        app.status = None;
                    }

                    KeyCode::Char('G') | KeyCode::End => {
                        match app.focus {
                            PaneFocus::Timeline => {
                                if !app.entries.is_empty() {
                                    app.list_state.select(Some(app.entries.len() - 1));
                                    app.detail_scroll = 0;
                                }
                            }
                            PaneFocus::Detail => {
                                app.detail_scroll = u16::MAX;
                            }
                        }
                        app.status = None;
                    }

                    KeyCode::Char('?') => app.show_help = !app.show_help,

                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn load_entries_from_str(input: &str, include_exec: bool) -> Result<Vec<Entry>> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let values = parse_json_values(trimmed)?;
    let mut entries = Vec::new();
    let mut exec_by_call_id: HashMap<String, usize> = HashMap::new();

    for value in values {
        let Some(event) = normalize_record(value)? else {
            continue;
        };

        match event {
            NormalizedEvent::Payload(event) => {
                if !include_exec && matches!(event, PayloadEvent::ExecCommandEnd { .. }) {
                    continue;
                }
                let index = entries.len();
                entries.push(to_entry(index, event));
            }
            NormalizedEvent::ExecToolCall {
                call_id,
                kind,
                name,
                input,
            } => {
                if !include_exec {
                    continue;
                }
                let index = entries.len();
                let entry = to_exec_tool_entry(index, call_id.clone(), &kind, &name, &input);
                entries.push(entry);
                if let Some(call_id) = call_id {
                    exec_by_call_id.insert(call_id, index);
                }
            }
            NormalizedEvent::ExecToolOutput { call_id, output } => {
                if let Some(index) = call_id
                    .as_deref()
                    .and_then(|call_id| exec_by_call_id.get(call_id))
                    .copied()
                {
                    append_exec_output(&mut entries[index], &output);
                }
            }
        }
    }

    Ok(entries)
}

fn parse_json_values(input: &str) -> Result<Vec<serde_json::Value>> {
    if input.starts_with('[') {
        let values: Vec<serde_json::Value> =
            serde_json::from_str(input).context("invalid json array input")?;
        return Ok(values);
    }

    let stream = serde_json::Deserializer::from_str(input).into_iter::<serde_json::Value>();

    let mut values = Vec::new();

    for value in stream {
        values.push(value.context("invalid json/jsonl stream object")?);
    }

    Ok(values)
}

fn normalize_record(value: serde_json::Value) -> Result<Option<NormalizedEvent>> {
    if let Ok(event) = serde_json::from_value::<PayloadEvent>(value.clone()) {
        return Ok(Some(NormalizedEvent::Payload(event)));
    }

    let raw: RawRecord = serde_json::from_value(value).context("invalid top-level codex record")?;

    match raw.r#type.as_deref() {
        Some("event_msg") => {
            let Some(payload) = raw.payload else {
                return Ok(None);
            };

            match serde_json::from_value::<PayloadEvent>(payload) {
                Ok(event) => Ok(Some(NormalizedEvent::Payload(event))),
                Err(_) => Ok(None),
            }
        }

        Some("response_item") => {
            let Some(payload) = raw.payload else {
                return Ok(None);
            };
            Ok(normalize_response_item(&payload))
        }

        _ => Ok(None),
    }
}

fn normalize_response_item(payload: &serde_json::Value) -> Option<NormalizedEvent> {
    match payload.get("type").and_then(serde_json::Value::as_str) {
        Some("message") => {
            let role = payload.get("role").and_then(serde_json::Value::as_str)?;
            let message = value_text(payload.get("content").unwrap_or(&serde_json::Value::Null));
            let phase = string_field(payload, "phase");

            match role {
                "user" => Some(NormalizedEvent::Payload(PayloadEvent::UserMessage {
                    message,
                    phase,
                })),
                "assistant" => Some(NormalizedEvent::Payload(PayloadEvent::AgentMessage {
                    message,
                    phase,
                })),
                _ => None,
            }
        }
        Some("custom_tool_call") | Some("function_call") if is_exec_tool_call(payload) => {
            Some(NormalizedEvent::ExecToolCall {
                call_id: string_field(payload, "call_id"),
                kind: payload
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool_call")
                    .to_string(),
                name: string_field(payload, "name").unwrap_or_else(|| "exec".to_string()),
                input: exec_tool_input(payload),
            })
        }
        Some("custom_tool_call_output") | Some("function_call_output") => {
            Some(NormalizedEvent::ExecToolOutput {
                call_id: string_field(payload, "call_id"),
                output: value_text(payload.get("output").unwrap_or(&serde_json::Value::Null)),
            })
        }
        _ => None,
    }
}

fn is_exec_tool_call(payload: &serde_json::Value) -> bool {
    matches!(
        payload.get("name").and_then(serde_json::Value::as_str),
        Some("exec") | Some("exec_command")
    )
}

fn exec_tool_input(payload: &serde_json::Value) -> String {
    let input = value_text(payload.get("input").unwrap_or(&serde_json::Value::Null));
    if input.trim().is_empty() {
        value_text(payload.get("arguments").unwrap_or(&serde_json::Value::Null))
    } else {
        input
    }
}

fn to_entry(index: usize, event: PayloadEvent) -> Entry {
    match event {
        PayloadEvent::UserMessage { message, phase } => Entry {
            index,
            kind: EntryKind::User,
            title: format!("#{index:04} USER{}", phase_suffix(phase.as_deref())),
            detail: message,
        },

        PayloadEvent::AgentMessage { message, phase } => Entry {
            index,
            kind: EntryKind::Agent,
            title: format!("#{index:04} AGENT{}", phase_suffix(phase.as_deref())),
            detail: message,
        },

        PayloadEvent::ExecCommandEnd {
            parsed_cmd,
            aggregated_output,
        } => {
            let command_summary = parsed_cmd
                .iter()
                .map(format_cmd_summary)
                .collect::<Vec<_>>()
                .join(" && ");

            let mut detail = String::new();

            if !parsed_cmd.is_empty() {
                detail.push_str("COMMANDS\n");
                detail.push_str("--------\n");
                for cmd in &parsed_cmd {
                    detail.push_str(&format_cmd_detail(cmd));
                    detail.push('\n');
                }
                detail.push('\n');
            }

            detail.push_str("OUTPUT\n");
            detail.push_str("------\n");
            detail.push_str(&aggregated_output);

            Entry {
                index,
                kind: EntryKind::Exec,
                title: format!("#{index:04} EXEC {command_summary}"),
                detail,
            }
        }
    }
}

fn to_exec_tool_entry(
    index: usize,
    call_id: Option<String>,
    kind: &str,
    name: &str,
    input: &str,
) -> Entry {
    let command = extract_exec_command(input).unwrap_or_else(|| input.to_string());
    let command_summary = truncate(&command.replace('\n', " "), 80);
    let mut detail = String::new();

    detail.push_str("COMMANDS\n");
    detail.push_str("--------\n");
    detail.push_str(&format!("type: {kind} | name: {name}"));
    if let Some(call_id) = call_id.as_deref() {
        detail.push_str(&format!(" | call_id: {call_id}"));
    }
    detail.push('\n');
    detail.push_str("$ ");
    detail.push_str(&command);
    detail.push('\n');

    if command != input {
        detail.push('\n');
        detail.push_str("RAW INPUT\n");
        detail.push_str("---------\n");
        detail.push_str(input);
        detail.push('\n');
    }

    detail.push('\n');
    detail.push_str("OUTPUT\n");
    detail.push_str("------\n");

    Entry {
        index,
        kind: EntryKind::Exec,
        title: format!("#{index:04} EXEC {command_summary}"),
        detail,
    }
}

fn append_exec_output(entry: &mut Entry, output: &str) {
    entry.detail.push_str(output);
}

fn phase_suffix(phase: Option<&str>) -> String {
    match phase {
        Some(phase) if !phase.is_empty() => format!(" [{phase}]"),
        _ => String::new(),
    }
}

fn string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .map(value_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(object) => object
            .get("text")
            .and_then(serde_json::Value::as_str)
            .or_else(|| object.get("output").and_then(serde_json::Value::as_str))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn extract_exec_command(input: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(input) {
        if let Some(cmd) = value.get("cmd").and_then(serde_json::Value::as_str) {
            return Some(cmd.to_string());
        }
    }

    let marker = "tools.exec_command(";
    let start = input.find(marker)? + marker.len();
    let json_object = json_object_at(&input[start..])?;
    serde_json::from_str::<serde_json::Value>(json_object)
        .ok()?
        .get("cmd")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn json_object_at(input: &str) -> Option<&str> {
    let start = input.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in input[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&input[start..start + offset + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }

    None
}

fn format_cmd_summary(cmd: &ParsedCmd) -> String {
    if let Some(name) = &cmd.name {
        if !name.is_empty() {
            return name.clone();
        }
    }

    if let Some(path) = &cmd.path {
        if !path.is_empty() {
            return path.clone();
        }
    }

    let one_line = cmd.cmd.replace('\n', " ");
    truncate(&one_line, 80)
}

fn format_cmd_detail(cmd: &ParsedCmd) -> String {
    let mut parts = Vec::new();

    if let Some(kind) = &cmd.r#type {
        parts.push(format!("type: {kind}"));
    }

    if let Some(name) = &cmd.name {
        parts.push(format!("name: {name}"));
    }

    if let Some(path) = &cmd.path {
        parts.push(format!("path: {path}"));
    }

    let meta = if parts.is_empty() {
        String::new()
    } else {
        format!("{}\n", parts.join(" | "))
    };

    format!("{meta}$ {}", cmd.cmd)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut out = String::new();

    for _ in 0..max_chars {
        match chars.next() {
            Some(ch) => out.push(ch),
            None => return out,
        }
    }

    if chars.next().is_some() {
        out.push('…');
    }

    out
}

fn render(frame: &mut Frame, app: &mut ReplayApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, root[0], app);

    match app.fullscreen {
        Fullscreen::None => {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
                .split(root[1]);

            render_list(frame, body[0], app);
            render_detail(frame, body[1], app);
        }
        Fullscreen::Timeline => {
            render_list(frame, root[1], app);
        }
        Fullscreen::Detail => {
            render_detail(frame, root[1], app);
        }
    }

    render_footer(frame, root[2], app);

    if app.show_help {
        render_help(frame);
    }
}

fn render_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &ReplayApp) {
    let selected = app
        .selected_index()
        .map(|i| format!("{}/{}", i + 1, app.entries.len()))
        .unwrap_or_else(|| "0/0".to_string());

    let header = Line::from(vec![
        Span::styled(
            " Codex Replay ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(selected, Style::default().fg(Color::Cyan)),
    ]);

    frame.render_widget(header, area);
}

fn render_list(frame: &mut Frame, area: ratatui::layout::Rect, app: &mut ReplayApp) {
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|entry| {
            let style = match entry.kind {
                EntryKind::User => Style::default().fg(Color::Green),
                EntryKind::Agent => Style::default().fg(Color::Blue),
                EntryKind::Exec => Style::default().fg(Color::Yellow),
            };

            ListItem::new(Line::from(vec![
                Span::styled(kind_label(&entry.kind), style.add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::raw(truncate(&entry.title, 120)),
            ]))
        })
        .collect();

    let focused = app.focus == PaneFocus::Timeline || app.fullscreen == Fullscreen::Timeline;

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let title = if focused {
        " Timeline [focus] "
    } else {
        " Timeline "
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

fn render_detail(frame: &mut Frame, area: ratatui::layout::Rect, app: &ReplayApp) {
    let Some(entry) = app.selected_entry() else {
        let empty = Paragraph::new("No entries")
            .block(Block::new().title(" Detail ").borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    };

    let focused = app.focus == PaneFocus::Detail || app.fullscreen == Fullscreen::Detail;

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let focus_suffix = if focused { " [focus]" } else { "" };
    let fullscreen_suffix = if app.fullscreen == Fullscreen::Detail {
        " [fullscreen]"
    } else {
        ""
    };

    let title = format!(" Detail{focus_suffix}{fullscreen_suffix}: {} ", entry.title);
    let text = detail_text(entry);

    let paragraph = Paragraph::new(text)
        .block(
            Block::new()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));

    frame.render_widget(paragraph, area);
}

fn detail_text(entry: &Entry) -> Text<'static> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("index: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(entry.index.to_string()),
    ]));

    lines.push(Line::from(vec![
        Span::styled("kind:  ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(kind_label(&entry.kind)),
    ]));

    lines.push(Line::raw(""));

    for line in entry.detail.lines() {
        lines.push(Line::raw(line.to_string()));
    }

    Text::from(lines)
}

fn kind_label(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::User => "USER ",
        EntryKind::Agent => "AGENT",
        EntryKind::Exec => "EXEC ",
    }
}

fn render_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &ReplayApp) {
    let focus = match app.focus {
        PaneFocus::Timeline => "timeline",
        PaneFocus::Detail => "detail",
    };

    let fullscreen = match app.fullscreen {
        Fullscreen::None => "",
        Fullscreen::Timeline => " | fullscreen: timeline",
        Fullscreen::Detail => " | fullscreen: detail",
    };

    let status = app
        .status
        .as_deref()
        .map(|s| format!(" | {s}"))
        .unwrap_or_default();

    let footer = Line::from(vec![
        Span::raw(format!(" focus: {focus}{fullscreen}{status} ")),
        Span::raw(" | Tab focus "),
        Span::raw(" 1/2 fullscreen "),
        Span::raw(" f fullscreen-focus "),
        Span::raw(" y copy-detail "),
        Span::raw(" q quit "),
        Span::raw(" ? help "),
    ])
    .gray();

    frame.render_widget(footer, area);
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(76, 82, frame.area());

    let help = Paragraph::new(vec![
        Line::styled(
            "Codex Replay TUI",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("Focus"),
        Line::raw("  Tab             switch focus between timeline/detail"),
        Line::raw(""),
        Line::raw("Timeline focus"),
        Line::raw("  j / Down        next event"),
        Line::raw("  k / Up          previous event"),
        Line::raw("  d / PageDown    next event"),
        Line::raw("  u / PageUp      previous event"),
        Line::raw("  g / Home        first event"),
        Line::raw("  G / End         last event"),
        Line::raw(""),
        Line::raw("Detail focus"),
        Line::raw("  j / Down        scroll detail down one line"),
        Line::raw("  k / Up          scroll detail up one line"),
        Line::raw("  d / PageDown    scroll detail down one page"),
        Line::raw("  u / PageUp      scroll detail up one page"),
        Line::raw("  g / Home        scroll detail to top"),
        Line::raw("  G / End         scroll detail to bottom"),
        Line::raw("  y               copy selected detail to clipboard"),
        Line::raw(""),
        Line::raw("Fullscreen"),
        Line::raw("  1               toggle timeline fullscreen"),
        Line::raw("  2               toggle detail fullscreen"),
        Line::raw("  f               toggle focused pane fullscreen"),
        Line::raw("  Esc             leave fullscreen/help or quit"),
        Line::raw(""),
        Line::raw("Other"),
        Line::raw("  ?               toggle help"),
        Line::raw("  q / Ctrl-C      quit"),
    ])
    .block(Block::new().title(" Help ").borders(Borders::ALL))
    .wrap(Wrap { trim: false });

    frame.render_widget(Clear, area);
    frame.render_widget(help, area);
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
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

    #[test]
    fn loads_raw_codex_jsonl_events() {
        let input = r#"{"type":"session_meta","payload":{"id":"ignored"}}
{"type":"event_msg","payload":{"type":"user_message","message":"hello","phase":"input"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"world"}}
{"type":"event_msg","payload":{"type":"exec_command_end","parsed_cmd":[{"type":"exec","cmd":"ls -la","name":"list"}],"aggregated_output":"done"}}"#;

        let entries = load_entries_from_str(input, true).expect("jsonl should parse");

        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[0].kind, EntryKind::User));
        assert_eq!(entries[0].title, "#0000 USER [input]");
        assert_eq!(entries[0].detail, "hello");
        assert!(matches!(entries[1].kind, EntryKind::Agent));
        assert!(matches!(entries[2].kind, EntryKind::Exec));
        assert!(entries[2].detail.contains("$ ls -la"));
        assert!(entries[2].detail.contains("done"));
    }

    #[test]
    fn loads_preprocessed_json_array_events() {
        let input = r#"[{"type":"user_message","message":"array input"}]"#;

        let entries = load_entries_from_str(input, false).expect("json array should parse");

        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, EntryKind::User));
        assert_eq!(entries[0].detail, "array input");
    }

    #[test]
    fn loads_response_item_exec_tool_calls_as_exec_entries() {
        let input = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"ignored by replay parser"}]}}
{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"call-1","name":"exec","input":"const r = await tools.exec_command({\"cmd\":\"pwd && ls\",\"workdir\":\"/repo/demo\"});\ntext(r.output);\n","status":"completed"}}
{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":[{"type":"input_text","text":"Script completed\nOutput:\n"},{"type":"input_text","text":"/repo/demo\nfile.txt\n"}]}}
{"type":"response_item","payload":{"type":"function_call_output","call_id":"unmatched","output":"not exec"}}"#;

        let entries = load_entries_from_str(input, true).expect("jsonl should parse");

        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].kind, EntryKind::User));
        assert_eq!(entries[0].detail, "ignored by replay parser");
        assert!(matches!(entries[1].kind, EntryKind::Exec));
        assert_eq!(entries[1].title, "#0001 EXEC pwd && ls");
        assert!(entries[1].detail.contains("$ pwd && ls"));
        assert!(entries[1].detail.contains("RAW INPUT"));
        assert!(entries[1].detail.contains("Script completed"));
        assert!(entries[1].detail.contains("file.txt"));
        assert!(!entries[1].detail.contains("not exec"));
    }

    #[test]
    fn loads_function_call_exec_command_as_exec_entry() {
        let input = r#"{"type":"response_item","payload":{"type":"function_call","call_id":"call-1","name":"exec_command","arguments":"{\"cmd\":\"git status --short\",\"workdir\":\"/repo/demo\"}"}}
{"type":"response_item","payload":{"type":"function_call_output","call_id":"call-1","output":" M README.md"}}"#;

        let entries = load_entries_from_str(input, true).expect("jsonl should parse");

        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, EntryKind::Exec));
        assert_eq!(entries[0].title, "#0000 EXEC git status --short");
        assert!(
            entries[0]
                .detail
                .contains("type: function_call | name: exec_command")
        );
        assert!(entries[0].detail.contains("$ git status --short"));
        assert!(entries[0].detail.contains(" M README.md"));
    }

    #[test]
    fn extracts_exec_command_from_tool_input() {
        let input =
            r#"const r = await tools.exec_command({"cmd":"printf \"hi\"","workdir":"/tmp"});"#;

        assert_eq!(
            extract_exec_command(input).as_deref(),
            Some(r#"printf "hi""#)
        );
        assert_eq!(
            extract_exec_command(r#"{"cmd":"cargo test"}"#).as_deref(),
            Some("cargo test")
        );
    }

    #[test]
    fn excludes_exec_entries_unless_requested() {
        let input = r#"{"type":"event_msg","payload":{"type":"user_message","message":"hello"}}
{"type":"event_msg","payload":{"type":"exec_command_end","parsed_cmd":[{"cmd":"pwd"}],"aggregated_output":"/tmp"}}"#;

        let entries = load_entries_from_str(input, false).expect("jsonl should parse");

        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, EntryKind::User));
    }

    #[test]
    fn reader_input_preserves_jsonl() {
        let mut input =
            br#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#.as_slice();

        assert_eq!(
            read_input_from_reader(&mut input).unwrap(),
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#
        );
    }
}
