//! The group picker and the channel picker, opened from the Settings tab.
//!
//! The group picker lists the groups; the channel picker lists the individual channels (all groups, one flat
//! list, filterable). Space picks; in the channel picker `e` gives a channel its own XMLTV id.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use tokio::runtime::Handle;

use super::input::Input;
use super::selected_style;
use crate::groups::normalize;
use crate::{config, CatalogEntry, Options, RawChannel};

struct Group {
    code: String,
    name: String,
    /// Lower-case, diacritics removed, for matching what the user saved.
    norm_name: String,
    /// Position of its first channel in the catalog (channels of a group are next to each other).
    start: usize,
    count: usize,
}

/// What the picker lists.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Groups,
    Channels,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Channel(usize),
}

enum Mode {
    Browse,
    Filter(Input),
    /// Typing the XMLTV id to write a channel under; the number is the channel's position in the catalog.
    NewId(Input, usize),
}

/// What happened because of a key.
pub struct Outcome {
    pub changed: bool,
    pub close: bool,
}

pub struct Picker {
    handle: Handle,
    cache_dir: String,
    strip: bool,
    kind: Kind,
    loading: Option<Receiver<Result<Vec<RawChannel>, String>>>,
    error: Option<String>,
    notice: Option<String>,
    catalog: Vec<CatalogEntry>,
    /// Per catalog entry: normalized name and XMLTV id, for the filter.
    keys: Vec<String>,
    /// Per catalog entry: position of its group in `groups`.
    group_of: Vec<usize>,
    groups: Vec<Group>,
    rows: Vec<Row>,
    state: ListState,
    filter: String,
    mode: Mode,
    id_width: usize,
}

impl Picker {
    pub fn open(handle: &Handle, cache_dir: String, strip: bool, kind: Kind) -> Self {
        let mut picker = Picker {
            handle: handle.clone(),
            cache_dir,
            strip,
            kind,
            loading: None,
            error: None,
            notice: None,
            catalog: Vec::new(),
            keys: Vec::new(),
            group_of: Vec::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            state: ListState::default(),
            filter: String::new(),
            mode: Mode::Browse,
            id_width: 0,
        };
        picker.start_loading(false);
        picker
    }

    pub fn hints(&self) -> &'static str {
        match (&self.mode, self.kind) {
            (Mode::Browse, Kind::Groups) => "Space pick a group · / filter · c clear groups · R reload the list · Esc back",
            (Mode::Browse, Kind::Channels) => {
                "Space pick · e custom id · / filter · c clear channels · R reload the list · Esc back"
            }
            (Mode::Filter(_), _) => "type to filter · Enter keep · Esc clear",
            (Mode::NewId(..), _) => "type the XMLTV id to write · empty = default id · Enter accept · Esc cancel",
        }
    }

    // ---------- loading the channel list ----------

    fn start_loading(&mut self, refresh: bool) {
        let (tx, rx) = mpsc::channel();
        let handle = self.handle.clone();
        let dir = self.cache_dir.clone();
        std::thread::spawn(move || {
            let result = handle.block_on(crate::load_raw_channels(&dir, refresh));
            let _ = tx.send(result);
        });
        self.loading = Some(rx);
        self.error = None;
    }

    /// Called regularly: picks up the channel list when it has arrived.
    pub fn tick(&mut self) {
        let Some(rx) = &self.loading else { return };
        match rx.try_recv() {
            Ok(Ok(raw)) => {
                self.loading = None;
                self.build_catalog(&raw);
            }
            Ok(Err(e)) => {
                self.loading = None;
                self.error = Some(e);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.loading = None;
                self.error = Some("Loading the channel list stopped unexpectedly.".to_string());
            }
        }
    }

    fn build_catalog(&mut self, raw: &[RawChannel]) {
        self.catalog = crate::channel_catalog(raw, self.strip);
        self.keys = self.catalog.iter().map(|e| normalize(&format!("{} {}", e.name, e.xml_id))).collect();
        self.id_width = self.catalog.iter().map(|e| e.xml_id.chars().count()).max().unwrap_or(0).min(40);

        self.groups.clear();
        self.group_of.clear();
        for (i, entry) in self.catalog.iter().enumerate() {
            let same = self.groups.last().is_some_and(|g| g.code == entry.code);
            if same {
                if let Some(g) = self.groups.last_mut() {
                    g.count += 1;
                }
            } else {
                self.groups.push(Group {
                    code: entry.code.clone(),
                    name: if entry.group.trim().is_empty() { "(no group)".to_string() } else { entry.group.clone() },
                    norm_name: normalize(&entry.group),
                    start: i,
                    count: 1,
                });
            }
            self.group_of.push(self.groups.len() - 1);
        }

        self.state = ListState::default();
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let q = normalize(&self.filter);
        let filtering = !q.is_empty();

        self.rows.clear();
        match self.kind {
            Kind::Groups => {
                for (gi, g) in self.groups.iter().enumerate() {
                    if filtering && !normalize(&format!("{} {}", g.name, g.code)).contains(&q) {
                        continue;
                    }
                    self.rows.push(Row::Group(gi));
                }
            }
            Kind::Channels => {
                for i in 0..self.catalog.len() {
                    if !filtering || self.keys[i].contains(&q) {
                        self.rows.push(Row::Channel(i));
                    }
                }
            }
        }

        let n = self.rows.len();
        let selected = self.state.selected().unwrap_or(0).min(n.saturating_sub(1));
        self.state.select(if n == 0 { None } else { Some(selected) });
    }

    // ---------- keys ----------

    pub fn on_key(&mut self, key: KeyEvent, o: &mut Options) -> Outcome {
        let mut out = Outcome { changed: false, close: false };

        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Filter(mut input) => match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.rebuild_rows();
                }
                KeyCode::Enter => {}
                _ => {
                    if input.handle(&key) {
                        self.filter = input.value();
                        self.rebuild_rows();
                    }
                    self.mode = Mode::Filter(input);
                }
            },
            Mode::NewId(mut input, index) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let text = input.value().trim().to_string();
                    if text.chars().any(char::is_whitespace) {
                        self.notice = Some("An XMLTV id must not contain spaces.".to_string());
                        self.mode = Mode::NewId(input, index);
                    } else if let Some(entry) = self.catalog.get(index) {
                        config::set_new_id(o, &entry.xml_id, if text.is_empty() { None } else { Some(text) });
                        out.changed = true;
                    }
                }
                _ => {
                    let _ = input.handle(&key);
                    self.mode = Mode::NewId(input, index);
                }
            },
            Mode::Browse => self.browse_key(key, o, &mut out),
        }

        out
    }

    fn current(&self) -> Option<Row> {
        self.state.selected().and_then(|i| self.rows.get(i)).copied()
    }

    fn move_by(&mut self, delta: i64) {
        let n = self.rows.len();
        if n == 0 {
            self.state.select(None);
            return;
        }
        let current = self.state.selected().unwrap_or(0) as i64;
        self.state.select(Some((current + delta).clamp(0, n as i64 - 1) as usize));
    }

    fn browse_key(&mut self, key: KeyEvent, o: &mut Options, out: &mut Outcome) {
        self.notice = None;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => out.close = true,
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(-1_000_000),
            KeyCode::End | KeyCode::Char('G') => self.move_by(1_000_000),
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle(o, out),
            KeyCode::Char('e') => match self.current() {
                Some(Row::Channel(ci)) => {
                    let existing = config::channel_request(o, &self.catalog[ci].xml_id)
                        .and_then(|r| r.new_id.clone())
                        .unwrap_or_default();
                    self.mode = Mode::NewId(Input::new(&existing), ci);
                }
                _ => self.notice = Some("Move to a channel first, then press e.".to_string()),
            },
            KeyCode::Char('/') => self.mode = Mode::Filter(Input::new(&self.filter)),
            KeyCode::Char('c') => {
                match self.kind {
                    Kind::Groups => o.groups = None,
                    Kind::Channels => o.channels.clear(),
                }
                out.changed = true;
                self.notice = Some("Cleared.".to_string());
            }
            KeyCode::Char('R') | KeyCode::F(5) => self.start_loading(true),
            _ => {}
        }
    }

    fn toggle(&mut self, o: &mut Options, out: &mut Outcome) {
        match self.current() {
            Some(Row::Group(gi)) => {
                let g = &self.groups[gi];
                config::toggle_group(o, &g.code, &g.name);
                out.changed = true;
            }
            Some(Row::Channel(ci)) => {
                config::toggle_channel(o, &self.catalog[ci].xml_id);
                out.changed = true;
            }
            None => {}
        }
    }

    // ---------- drawing ----------

    pub fn draw(&mut self, f: &mut Frame, area: Rect, o: &Options) {
        let what = match self.kind {
            Kind::Groups => "Channel groups",
            Kind::Channels => "Individual channels",
        };
        let title = format!(" {what}: {} ", config::selection_summary(o));
        let block = Block::bordered().title(title).border_style(Style::new().fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let [list_area, bottom] = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(inner);

        if self.loading.is_some() {
            f.render_widget(Paragraph::new("Loading the channel list ..."), list_area);
            return;
        }
        if let Some(error) = &self.error {
            let lines = vec![
                Line::styled(error.clone(), Style::new().fg(Color::Red)),
                Line::raw(""),
                Line::raw("Press R to try again, or Esc to go back."),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list_area);
            return;
        }

        let wanted: Vec<String> = o.groups.iter().flatten().map(|g| normalize(g)).collect();
        let group_picked = |g: &Group| wanted.iter().any(|w| *w == g.code || *w == g.norm_name);
        let filtering = !self.filter.is_empty();

        let items: Vec<ListItem<'static>> = self
            .rows
            .iter()
            .map(|row| match *row {
                Row::Group(gi) => {
                    let g = &self.groups[gi];
                    let text = format!(
                        "[{}] {} [{}]  {} channels",
                        if group_picked(g) { "x" } else { " " },
                        g.name,
                        g.code,
                        g.count
                    );
                    ListItem::new(Line::styled(text, Style::new().fg(Color::Cyan)))
                }
                Row::Channel(ci) => {
                    let e = &self.catalog[ci];
                    let request = config::channel_request(o, &e.xml_id);
                    let mark = if request.is_some() {
                        "x"
                    } else if group_picked(&self.groups[self.group_of[ci]]) {
                        "g"
                    } else {
                        " "
                    };
                    let mut spans = vec![Span::raw(format!("[{mark}] {:<w$}  {}", e.xml_id, e.name, w = self.id_width))];
                    if let Some(new_id) = request.and_then(|r| r.new_id.as_deref()) {
                        spans.push(Span::styled(format!("  → {new_id}"), Style::new().fg(Color::Yellow)));
                    }
                    let style = if mark == "g" { Style::new().fg(Color::DarkGray) } else { Style::new() };
                    ListItem::new(Line::from(spans)).style(style)
                }
            })
            .collect();

        if items.is_empty() {
            f.render_widget(Paragraph::new("No channel matches the filter."), list_area);
        } else {
            let list = List::new(items).highlight_style(selected_style()).highlight_symbol("");
            f.render_stateful_widget(list, list_area, &mut self.state);
        }

        // the line(s) below the list: an input field, a notice, or the legend
        let lines: Vec<Line<'static>> = match &self.mode {
            Mode::Filter(input) => {
                let mut spans = vec![Span::styled("Filter: ", Style::new().fg(Color::Yellow))];
                spans.extend(input.spans(bottom.width.saturating_sub(10) as usize));
                vec![Line::from(spans)]
            }
            Mode::NewId(input, ci) => {
                let id = self.catalog.get(*ci).map(|e| e.xml_id.clone()).unwrap_or_default();
                let mut spans = vec![Span::styled(format!("{id} is written as: "), Style::new().fg(Color::Yellow))];
                spans.extend(input.spans(bottom.width.saturating_sub(id.chars().count() as u16 + 20) as usize));
                vec![Line::from(spans)]
            }
            Mode::Browse => {
                let mut v = Vec::new();
                if let Some(notice) = &self.notice {
                    v.push(Line::styled(notice.clone(), Style::new().fg(Color::Yellow)));
                } else if filtering {
                    v.push(Line::styled(format!("Filter: {}", self.filter), Style::new().fg(Color::Yellow)));
                }
                v.push(Line::styled(
                    match self.kind {
                        Kind::Groups => "[x] picked   no group and no channel picked means every channel is written",
                        Kind::Channels => "[x] picked   [g] comes with a picked group   picked groups and channels are all written",
                    },
                    Style::new().fg(Color::DarkGray),
                ));
                v
            }
        };
        f.render_widget(Paragraph::new(lines), bottom);
    }
}
