//! The "Guide" tab: channels on the left, the programmes of one day on the right, details below.

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::input::Input;
use super::{border_style, centered, fit, selected_style, Action};
use crate::groups::normalize;
use crate::guide::{self, Guide, GuideChannel, GuideProgramme};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Channels,
    Programmes,
    Details,
}

struct Search {
    input: Input,
    /// (channel, programme) positions in the guide
    hits: Vec<(usize, usize)>,
    state: ListState,
}

enum Mode {
    Normal,
    Filter(Input),
    Search(Search),
}

pub struct GuideTab {
    path: PathBuf,
    guide: Option<Guide>,
    error: Option<String>,
    loading: bool,
    day: Option<NaiveDate>,
    /// Positions (in guide.channels) of the channels that pass the filter.
    visible: Vec<usize>,
    filter: String,
    channels: ListState,
    programmes: ListState,
    focus: Focus,
    scroll: u16,
    max_scroll: u16,
    mode: Mode,
}

/// Today's date where the guide is from.
fn today() -> NaiveDate {
    Utc::now().with_timezone(&chrono_tz::Europe::Prague).date_naive()
}

fn initial_day(guide: &Guide) -> Option<NaiveDate> {
    let today = today();
    guide.dates.iter().copied().find(|d| *d >= today).or_else(|| guide.dates.last().copied())
}

/// Moves a list selection by `delta` rows, staying inside the list.
fn step(state: &mut ListState, len: usize, delta: i64) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as i64;
    let next = (current + delta).clamp(0, len as i64 - 1);
    state.select(Some(next as usize));
}

fn selection(len: usize) -> ListState {
    let mut state = ListState::default();
    state.select(if len == 0 { None } else { Some(0) });
    state
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl GuideTab {
    pub fn new(path: PathBuf) -> Self {
        GuideTab {
            path,
            guide: None,
            error: None,
            loading: true,
            day: None,
            visible: Vec::new(),
            filter: String::new(),
            channels: ListState::default(),
            programmes: ListState::default(),
            focus: Focus::Channels,
            scroll: 0,
            max_scroll: 0,
            mode: Mode::Normal,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
    }

    pub fn captures_text(&self) -> bool {
        !matches!(self.mode, Mode::Normal)
    }

    pub fn hints(&self) -> &'static str {
        match &self.mode {
            Mode::Normal => {
                "Tab pane · ↑↓ move · ←→ day · n now · / search · f filter channels · r reload · ? help · q quit"
            }
            Mode::Filter(_) => "type to filter the channels · Enter keep the filter · Esc clear it",
            Mode::Search(_) => "type to search · ↑↓ choose · Enter go to the programme · Esc close",
        }
    }

    // ---------- loading ----------

    /// Reads the guide file again, keeping the selected channel and day where possible.
    pub fn reload(&mut self) {
        let keep_channel = self.selected_channel().map(|c| c.id.clone());
        let keep_day = self.day;
        self.loading = false;

        match guide::load(&self.path) {
            Ok(g) => {
                let day = keep_day.filter(|d| g.dates.contains(d)).or_else(|| initial_day(&g));
                self.guide = Some(g);
                self.error = None;
                self.day = day;
                self.apply_filter();

                if let Some(id) = keep_channel {
                    let position = self
                        .guide
                        .as_ref()
                        .and_then(|g| self.visible.iter().position(|&i| g.channels[i].id == id));
                    if let Some(position) = position {
                        self.channels.select(Some(position));
                    }
                }
                self.select_programme();
            }
            Err(e) => {
                self.guide = None;
                self.visible.clear();
                self.error = Some(e);
            }
        }
    }

    fn apply_filter(&mut self) {
        let Some(g) = &self.guide else {
            self.visible.clear();
            self.channels.select(None);
            return;
        };

        let q = normalize(&self.filter);
        self.visible = g
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| q.is_empty() || c.search.contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.channels = selection(self.visible.len());
    }

    // ---------- what is selected ----------

    fn selected_channel_index(&self) -> Option<usize> {
        self.channels.selected().and_then(|i| self.visible.get(i)).copied()
    }

    fn selected_channel(&self) -> Option<&GuideChannel> {
        let g = self.guide.as_ref()?;
        g.channels.get(self.selected_channel_index()?)
    }

    /// Positions (in the channel's programmes) of what is shown for the selected channel and day.
    fn day_list(&self) -> Vec<usize> {
        match (self.selected_channel(), self.day) {
            (Some(channel), Some(day)) => channel.on(day),
            _ => Vec::new(),
        }
    }

    fn selected_programme(&self) -> Option<(&GuideChannel, &GuideProgramme)> {
        let channel = self.selected_channel()?;
        let position = *self.day_list().get(self.programmes.selected()?)?;
        Some((channel, channel.programmes.get(position)?))
    }

    /// On today's list: the programme that is on now, or the next one. None for other days.
    fn current_position(&self, list: &[usize]) -> Option<usize> {
        if self.day != Some(today()) {
            return None;
        }
        let channel = self.selected_channel()?;
        let now = Utc::now().timestamp();
        list.iter().position(|&i| channel.end(i).map_or(true, |end| end.timestamp() > now))
    }

    /// Selects the programme that is on now (today) or the first one (other days).
    fn select_programme(&mut self) {
        self.scroll = 0;
        let list = self.day_list();
        let mut state = ListState::default();
        if !list.is_empty() {
            state.select(Some(self.current_position(&list).unwrap_or(0)));
        }
        self.programmes = state;
    }

    // ---------- keys ----------

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.key_normal(key),
            Mode::Filter(input) => self.key_filter(key, input),
            Mode::Search(search) => self.key_search(key, search),
        }
    }

    fn key_normal(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Channels => Focus::Programmes,
                    Focus::Programmes => Focus::Details,
                    Focus::Details => Focus::Channels,
                }
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Channels => Focus::Details,
                    Focus::Programmes => Focus::Channels,
                    Focus::Details => Focus::Programmes,
                }
            }
            KeyCode::Enter => {
                self.focus = match self.focus {
                    Focus::Channels => Focus::Programmes,
                    Focus::Programmes | Focus::Details => Focus::Details,
                }
            }
            KeyCode::Esc => {
                self.focus = match self.focus {
                    Focus::Details => Focus::Programmes,
                    _ => Focus::Channels,
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(-1_000_000),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(1_000_000),
            KeyCode::Left | KeyCode::Char('h') => self.move_day(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_day(1),
            KeyCode::Char('n') => self.go_now(),
            KeyCode::Char('r') => return Action::ReloadGuide,
            KeyCode::Char('/') => {
                self.mode = Mode::Search(Search { input: Input::new(""), hits: Vec::new(), state: ListState::default() });
            }
            KeyCode::Char('f') => {
                self.mode = Mode::Filter(Input::new(&self.filter));
            }
            _ => {}
        }
        Action::None
    }

    fn key_filter(&mut self, key: KeyEvent, mut input: Input) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.apply_filter();
                self.select_programme();
            }
            KeyCode::Enter => {}
            _ => {
                if input.handle(&key) {
                    self.filter = input.value();
                    self.apply_filter();
                    self.select_programme();
                }
                self.mode = Mode::Filter(input);
            }
        }
        Action::None
    }

    fn key_search(&mut self, key: KeyEvent, mut search: Search) -> Action {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let hit = search.state.selected().and_then(|i| search.hits.get(i)).copied();
                if let Some((channel, programme)) = hit {
                    self.jump_to(channel, programme);
                } else {
                    self.mode = Mode::Search(search);
                }
            }
            KeyCode::Up => {
                step(&mut search.state, search.hits.len(), -1);
                self.mode = Mode::Search(search);
            }
            KeyCode::Down => {
                step(&mut search.state, search.hits.len(), 1);
                self.mode = Mode::Search(search);
            }
            KeyCode::PageUp => {
                step(&mut search.state, search.hits.len(), -10);
                self.mode = Mode::Search(search);
            }
            KeyCode::PageDown => {
                step(&mut search.state, search.hits.len(), 10);
                self.mode = Mode::Search(search);
            }
            _ => {
                if search.input.handle(&key) {
                    search.hits = match &self.guide {
                        Some(g) => g.search(&search.input.value(), 500),
                        None => Vec::new(),
                    };
                    search.state = selection(search.hits.len());
                }
                self.mode = Mode::Search(search);
            }
        }
        Action::None
    }

    fn move_selection(&mut self, delta: i64) {
        match self.focus {
            Focus::Channels => {
                step(&mut self.channels, self.visible.len(), delta);
                self.select_programme();
            }
            Focus::Programmes => {
                let len = self.day_list().len();
                step(&mut self.programmes, len, delta);
                self.scroll = 0;
            }
            Focus::Details => {
                let next = (i64::from(self.scroll) + delta).clamp(0, i64::from(self.max_scroll));
                self.scroll = next as u16;
            }
        }
    }

    fn move_day(&mut self, delta: i64) {
        let new_day = {
            let Some(g) = &self.guide else { return };
            let Some(day) = self.day else { return };
            if g.dates.is_empty() {
                return;
            }
            let position = g.dates.iter().position(|d| *d == day).unwrap_or(0) as i64;
            let next = (position + delta).clamp(0, g.dates.len() as i64 - 1) as usize;
            g.dates.get(next).copied()
        };
        if new_day != self.day {
            self.day = new_day;
            self.select_programme();
        }
    }

    fn go_now(&mut self) {
        let day = self.guide.as_ref().and_then(initial_day);
        if day.is_some() {
            self.day = day;
        }
        self.focus = Focus::Programmes;
        self.select_programme();
    }

    /// Shows a search result: its channel, its day, and the programme itself.
    fn jump_to(&mut self, channel: usize, programme: usize) {
        let day = {
            let Some(g) = &self.guide else { return };
            let Some(p) = g.channels.get(channel).and_then(|c| c.programmes.get(programme)) else { return };
            p.start.date_naive()
        };

        if !self.visible.contains(&channel) {
            self.filter.clear();
            self.apply_filter();
        }
        if let Some(position) = self.visible.iter().position(|&c| c == channel) {
            self.channels.select(Some(position));
        }
        self.day = Some(day);

        let list = self.day_list();
        let mut state = ListState::default();
        if !list.is_empty() {
            state.select(Some(list.iter().position(|&i| i == programme).unwrap_or(0)));
        }
        self.programmes = state;
        self.scroll = 0;
        self.focus = Focus::Programmes;
    }

    // ---------- drawing ----------

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        if self.loading {
            let text = format!("Loading {} ...", self.path.display());
            f.render_widget(Paragraph::new(text).block(Block::bordered().title(" Guide ")), area);
            return;
        }

        if self.guide.is_none() {
            let mut lines = vec![
                Line::styled("No guide to show.", Style::new().add_modifier(Modifier::BOLD)),
                Line::raw(""),
            ];
            if let Some(error) = &self.error {
                lines.push(Line::styled(error.clone(), Style::new().fg(Color::Red)));
                lines.push(Line::raw(""));
            }
            lines.push(Line::raw(format!("Looked for: {}", self.path.display())));
            lines.push(Line::raw("Press 3, then s, to download the guide. Press r to look again."));
            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" Guide ")),
                area,
            );
            return;
        }

        let narrow = area.width < 90;
        let [left, right] =
            Layout::horizontal([Constraint::Length(if narrow { 22 } else { 30 }), Constraint::Min(20)]).areas(area);
        let [head, programmes, details] =
            Layout::vertical([Constraint::Length(3), Constraint::Percentage(50), Constraint::Min(4)]).areas(right);

        self.draw_channels(f, left);
        self.draw_header(f, head);
        self.draw_programmes(f, programmes);
        self.draw_details(f, details);

        if let (Mode::Search(search), Some(g)) = (&mut self.mode, &self.guide) {
            draw_search(f, area, search, g);
        }
    }

    fn draw_channels(&mut self, f: &mut Frame, area: Rect) {
        let (list_area, filter_area) = if matches!(self.mode, Mode::Filter(_)) {
            let [top, bottom] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);
            (top, Some(bottom))
        } else {
            (area, None)
        };

        let width = list_area.width.saturating_sub(4) as usize;
        let (items, total): (Vec<ListItem<'static>>, usize) = match &self.guide {
            Some(g) => (
                self.visible
                    .iter()
                    .map(|&i| ListItem::new(fit(&g.channels[i].name, width)))
                    .collect(),
                g.channels.len(),
            ),
            None => (Vec::new(), 0),
        };

        let mut title = format!(" Channels {}/{} ", self.visible.len(), total);
        if !self.filter.is_empty() {
            title = format!(" Channels {}/{} \"{}\" ", self.visible.len(), total, fit(&self.filter, 10));
        }

        let list = List::new(items)
            .block(Block::bordered().title(title).border_style(border_style(self.focus == Focus::Channels)))
            .highlight_style(selected_style())
            .highlight_symbol("▸ ");
        f.render_stateful_widget(list, list_area, &mut self.channels);

        if let (Some(filter_area), Mode::Filter(input)) = (filter_area, &self.mode) {
            let block = Block::bordered().title(" Filter ").border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(filter_area);
            f.render_widget(block, filter_area);
            f.render_widget(Paragraph::new(Line::from(input.spans(inner.width as usize))), inner);
        }
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        match self.day {
            Some(day) => {
                let position = self
                    .guide
                    .as_ref()
                    .and_then(|g| g.dates.iter().position(|d| *d == day))
                    .map_or(0, |p| p + 1);
                let count = self.guide.as_ref().map_or(0, |g| g.dates.len());
                let total = self.guide.as_ref().map_or(0, |g| g.programme_count);
                spans.push(Span::styled(
                    format!("◂ {} ▸", day.format("%a %d %b %Y")),
                    Style::new().add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("  day {position}/{count}, {total} programmes in the file"),
                    Style::new().fg(Color::DarkGray),
                ));
                if day == today() {
                    spans.push(Span::styled("  today", Style::new().fg(Color::Green)));
                }
            }
            None => spans.push(Span::raw("The guide has no programmes")),
        }
        if let Some(channel) = self.selected_channel() {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(channel.name.clone(), Style::new().fg(Color::Cyan)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).block(Block::bordered()), area);
    }

    fn draw_programmes(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Programmes;
        let list = self.day_list();
        let now = Utc::now().timestamp();

        let items: Vec<ListItem<'static>> = match self.selected_channel() {
            Some(channel) => list
                .iter()
                .map(|&i| {
                    let p = &channel.programmes[i];
                    let end = channel.end(i).map(|e| e.timestamp());
                    let is_now = p.start.timestamp() <= now && end.is_some_and(|e| now < e);
                    let is_past = end.is_some_and(|e| e <= now);

                    let mut text = format!("{} {}  {}", if is_now { ">" } else { " " }, p.start.format("%H:%M"), p.title);
                    let tags = p.tags();
                    if !tags.is_empty() {
                        text.push_str(&format!("  [{}]", tags.join(" ")));
                    }
                    let style = if is_now {
                        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else if is_past {
                        Style::new().fg(Color::DarkGray)
                    } else {
                        Style::new()
                    };
                    ListItem::new(Line::styled(text, style))
                })
                .collect(),
            None => Vec::new(),
        };

        let block = Block::bordered()
            .title(format!(" Programmes ({}) ", list.len()))
            .border_style(border_style(focused));

        if items.is_empty() {
            f.render_widget(Paragraph::new("Nothing on this day.").block(block), area);
            return;
        }

        let widget = List::new(items).block(block).highlight_style(selected_style()).highlight_symbol("");
        f.render_stateful_widget(widget, area, &mut self.programmes);
    }

    fn draw_details(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Details;
        let lines: Vec<Line<'static>> = match self.selected_programme() {
            Some((channel, programme)) => detail_lines(channel, programme),
            None => vec![Line::raw("Select a programme.")],
        };

        // Roughly how far the text can scroll (wrapped lines counted by width).
        let inner_width = area.width.saturating_sub(2).max(1) as usize;
        let inner_height = area.height.saturating_sub(2) as usize;
        let rows: usize = lines.iter().map(|l| l.width().max(1).div_ceil(inner_width)).sum();
        self.max_scroll = rows.saturating_sub(inner_height).min(u16::MAX as usize) as u16;
        self.scroll = self.scroll.min(self.max_scroll);

        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(Block::bordered().title(" Details ").border_style(border_style(focused))),
            area,
        );
    }
}

/// The text of the details pane.
fn detail_lines(channel: &GuideChannel, p: &GuideProgramme) -> Vec<Line<'static>> {
    let label = Style::new().fg(Color::Cyan);
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::styled(p.title.clone(), Style::new().add_modifier(Modifier::BOLD)));
    if let Some(sub) = &p.sub_title {
        lines.push(Line::styled(sub.clone(), Style::new().add_modifier(Modifier::ITALIC)));
    }

    // when, and how far along it is
    let position = channel.programmes.iter().position(|x| std::ptr::eq(x, p));
    let end = position.and_then(|i| channel.end(i));
    let mut when = format!("{}", p.start.format("%a %d.%m. %H:%M"));
    if let Some(end) = end {
        when.push_str(&format!(" – {}", end.format("%H:%M")));
        let minutes = (end - p.start).num_minutes();
        if minutes > 0 {
            when.push_str(&format!("  ({minutes} min)"));
        }
    }
    let mut spans = vec![Span::styled("When: ", label), Span::raw(when)];
    if let Some(end) = end {
        let now = Utc::now().timestamp();
        let (start, stop) = (p.start.timestamp(), end.timestamp());
        if start <= now && now < stop && stop > start {
            let percent = (now - start) * 100 / (stop - start);
            spans.push(Span::styled(format!("   on now, {percent}% done"), Style::new().fg(Color::Green)));
        }
    }
    lines.push(Line::from(spans));

    let mut facts: Vec<String> = Vec::new();
    if !p.categories.is_empty() {
        facts.push(p.categories.join(", "));
    }
    if let Some(year) = &p.year {
        facts.push(year.clone());
    }
    if let Some(country) = &p.country {
        facts.push(country.clone());
    }
    if let Some(length) = &p.length {
        facts.push(format!("{length} min"));
    }
    if let Some(episode) = &p.episode {
        facts.push(format!("episode {episode}"));
    }
    if let Some(rating) = &p.rating {
        facts.push(format!("rating {rating}"));
    }
    let tags = p.tags();
    if !tags.is_empty() {
        facts.push(tags.join(" "));
    }
    if !facts.is_empty() {
        lines.push(Line::from(vec![Span::styled("Info: ", label), Span::raw(facts.join(" · "))]));
    }

    if let Some(description) = &p.description {
        lines.push(Line::raw(""));
        for paragraph in description.lines() {
            lines.push(Line::raw(paragraph.trim().to_string()));
        }
    }

    if !p.credits.is_empty() {
        lines.push(Line::raw(""));
        let mut kinds: Vec<&str> = Vec::new();
        for (kind, _) in &p.credits {
            if !kinds.contains(&kind.as_str()) {
                kinds.push(kind.as_str());
            }
        }
        for kind in kinds {
            let names: Vec<&str> =
                p.credits.iter().filter(|(k, _)| k.as_str() == kind).map(|(_, name)| name.as_str()).collect();
            lines.push(Line::from(vec![
                Span::styled(format!("{}: ", capitalize(kind)), label),
                Span::raw(names.join(", ")),
            ]));
        }
    }

    lines
}

fn draw_search(f: &mut Frame, area: Rect, search: &mut Search, guide: &Guide) {
    let popup = centered(area, (area.width * 9 / 10).max(30), (area.height * 8 / 10).max(8));
    f.render_widget(Clear, popup);

    let title = format!(" Search programmes ({} found) ", search.hits.len());
    let block = Block::bordered().title(title).border_style(Style::new().fg(Color::Yellow));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let [input_area, list_area] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);

    let mut spans = vec![Span::styled("/ ", Style::new().fg(Color::Yellow))];
    spans.extend(search.input.spans(input_area.width.saturating_sub(2) as usize));
    f.render_widget(Paragraph::new(Line::from(spans)), input_area);

    let items: Vec<ListItem<'static>> = search
        .hits
        .iter()
        .map(|&(c, p)| {
            let channel = &guide.channels[c];
            let programme = &channel.programmes[p];
            ListItem::new(format!(
                "{}  {:<16}  {}",
                programme.start.format("%a %d.%m. %H:%M"),
                fit(&channel.name, 16),
                programme.title
            ))
        })
        .collect();

    if items.is_empty() {
        let hint = if search.input.value().is_empty() { "Type part of a title." } else { "No programme matches." };
        f.render_widget(Paragraph::new(Line::styled(hint, Style::new().fg(Color::DarkGray))), list_area);
    } else {
        let list = List::new(items).highlight_style(selected_style()).highlight_symbol("▸ ");
        f.render_stateful_widget(list, list_area, &mut search.state);
    }
}
