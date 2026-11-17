//! Terminal view over a resolved AI Catalog tree.
//!
//! `render` draws from a [`ResolvedCatalog`] and a [`ViewState`] only. Every string that comes
//! from a document, a report or a URL reaches the buffer as [`DisplayText`]; `data` is shown only
//! as the text of its canonical JSON, whatever the entry `type` is (#embedded-content-safety).

use std::fmt;
use std::io::{self, IsTerminal};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog_client::{Child, DisplayText, EntryRow, ResolvedCatalog, SkipReason};

const DETAIL_HEIGHT: u16 = 12;

/// What the session does after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    Continue,
    Quit,
}

/// The selection over every entry row of the tree, in the order `render` draws them.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    selected: usize,
}

impl ViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Up/Down and `k`/`j` move the selection within bounds; `q`, Esc and Ctrl-C quit; every
    /// other key changes nothing.
    pub fn on_key(&mut self, key: KeyEvent, catalog: &ResolvedCatalog) -> Action {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let last = count_rows(catalog).saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

fn count_rows(node: &ResolvedCatalog) -> usize {
    node.rows.len()
        + node
            .children
            .iter()
            .map(|n| match &n.child {
                Child::Catalog(c) => count_rows(c),
                _ => 0,
            })
            .sum::<usize>()
}

struct Drawn<'a> {
    lines: Vec<Line<'a>>,
    selected_line: usize,
    detail: Option<(&'a ResolvedCatalog, &'a EntryRow)>,
    next_row: usize,
}

fn skip_label(reason: &SkipReason) -> &'static str {
    match reason {
        SkipReason::Budget => "budget",
        SkipReason::Depth => "depth",
        SkipReason::Cycle => "cycle",
        SkipReason::Unresolvable => "unresolvable",
        SkipReason::Fetch(_) => "fetch",
        _ => "other",
    }
}

fn shown(text: &str) -> String {
    DisplayText::new(text).to_string()
}

fn draw_node<'a>(node: &'a ResolvedCatalog, depth: usize, selected: usize, out: &mut Drawn<'a>) {
    let pad = "  ".repeat(depth);
    let source = node.base.as_deref().map_or("file".to_string(), shown);
    let host = node.document.host.as_ref().map_or(String::new(), |h| {
        format!("  host: {}", shown(&h.display_name))
    });
    out.lines.push(Line::styled(
        format!(
            "{pad}catalog {source}{host}  conformance: {:?}",
            node.report.conformance()
        ),
        Style::new().add_modifier(Modifier::BOLD),
    ));
    for row in &node.rows {
        let version = row
            .version
            .as_deref()
            .map_or(String::new(), |v| format!("  {}", shown(v)));
        let is_selected = out.next_row == selected;
        let marker = if is_selected { ">" } else { " " };
        let style = if is_selected {
            out.selected_line = out.lines.len();
            out.detail = Some((node, row));
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        out.lines.push(Line::styled(
            format!("{pad}{marker} {}{version}", shown(&row.name)),
            style,
        ));
        out.next_row += 1;
    }
    for nested in &node.children {
        if let Child::Skipped { url, reason } = &nested.child {
            let cause = match reason {
                SkipReason::Fetch(e) => format!(": {}", shown(&e.to_string())),
                _ => String::new(),
            };
            out.lines.push(Line::raw(format!(
                "{pad}  skipped {} {}{cause}",
                skip_label(reason),
                shown(url)
            )));
        }
    }
    for w in &node.warnings {
        out.lines.push(Line::raw(format!(
            "{pad}  warning {}: {}",
            w.code(),
            shown(&w.to_string())
        )));
    }
    for d in node.report.diagnostics() {
        out.lines.push(Line::raw(format!(
            "{pad}  {} {} {}",
            format!("{:?}", d.severity).to_lowercase(),
            d.code,
            shown(&d.path.to_string())
        )));
    }
    for nested in &node.children {
        if let Child::Catalog(c) = &nested.child {
            draw_node(c, depth + 1, selected, out);
        }
    }
}

fn detail_lines(node: &ResolvedCatalog, row: &EntryRow) -> Vec<Line<'static>> {
    let entry = &node.document.entries[row.index];
    let none = || "-".to_string();
    let mut lines = vec![
        Line::raw(format!("identifier: {}", shown(entry.identifier.as_str()))),
        Line::raw(format!("type: {}", shown(entry.artifact_type.as_str()))),
        Line::raw(format!(
            "version: {}",
            row.version.as_deref().map_or_else(none, shown)
        )),
        Line::raw(format!("display name: {}", shown(&row.name))),
        Line::raw(format!(
            "description: {}",
            row.description.as_deref().map_or_else(none, shown)
        )),
    ];
    lines.push(match &entry.source {
        ArtifactSource::Url(u) => Line::raw(format!("url: {}", shown(u))),
        ArtifactSource::Data(j) => match j.to_canonical_vec() {
            Ok(bytes) => Line::raw(format!("data: {}", shown(&String::from_utf8_lossy(&bytes)))),
            Err(CanonicalError::NotIJson { pointer }) => Line::raw(format!(
                "data not shown: not I-JSON at {}",
                shown(&pointer.to_string())
            )),
            Err(e) => Line::raw(format!("data not shown: {}", shown(&e.to_string()))),
        },
    });
    lines.push(Line::raw(format!(
        "trust manifests (not verified): {}",
        entry.trust_manifests.as_ref().map_or(0, Vec::len)
    )));
    lines
}

/// Draws the whole tree and the selected entry's detail. Pure: reads only its arguments.
pub fn render(catalog: &ResolvedCatalog, state: &ViewState, frame: &mut Frame) {
    let mut out = Drawn {
        lines: Vec::new(),
        selected_line: 0,
        detail: None,
        next_row: 0,
    };
    draw_node(catalog, 0, state.selected, &mut out);

    let [list_area, detail_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(DETAIL_HEIGHT)])
            .areas(frame.area());
    let height = usize::from(list_area.height);
    let scroll = (out.selected_line + 1).saturating_sub(height);
    let scroll = u16::try_from(scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(out.lines).scroll((scroll, 0)), list_area);

    let detail = out
        .detail
        .map_or_else(Vec::new, |(node, row)| detail_lines(node, row));
    frame.render_widget(
        Paragraph::new(detail).block(Block::bordered().title("entry")),
        detail_area,
    );
}

/// Why a terminal session did not run or failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunError {
    NotATerminal,
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::NotATerminal => f.write_str("stdin and stdout must both be terminals"),
            RunError::Io(e) => write!(f, "terminal error: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<io::Error> for RunError {
    fn from(e: io::Error) -> Self {
        RunError::Io(e)
    }
}

/// Shows the catalog until a quit key. Returns `NotATerminal` before touching the terminal
/// unless stdin and stdout are both terminals.
pub fn run(catalog: ResolvedCatalog) -> Result<(), RunError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(RunError::NotATerminal);
    }
    let mut terminal = ratatui::init();
    let result = session(&mut terminal, &catalog);
    ratatui::restore();
    result
}

fn session(
    terminal: &mut ratatui::DefaultTerminal,
    catalog: &ResolvedCatalog,
) -> Result<(), RunError> {
    let mut state = ViewState::new();
    loop {
        terminal.draw(|f| render(catalog, &state, f))?;
        if let Event::Key(key) = crossterm::event::read()?
            && key.kind == KeyEventKind::Press
            && state.on_key(key, catalog) == Action::Quit
        {
            return Ok(());
        }
    }
}
