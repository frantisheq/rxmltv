//! The "Guide" tab: channels on the left, the programmes of one day on the right, details below.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::input::Input;
use super::{border_style, centered, fit, parse_color, selected_style, Action};
use crate::groups::normalize;
use crate::config::UiOptions;
use crate::guide::{self, Guide, GuideChannel, GuideProgramme};
use crate::store::{self, Favorites};

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
    /// All channels (positions in guide.channels) in the order the user wants them.
    order: Vec<usize>,
    order_path: PathBuf,
    /// Positions (in guide.channels) of the channels that pass the filter, in that order.
    visible: Vec<usize>,
    filter: String,
    channels: ListState,
    programmes: ListState,
    focus: Focus,
    scroll: u16,
    max_scroll: u16,
    mode: Mode,
    /// Channels as columns (the newspaper layout) instead of a channel list and a programme list.
    grid: bool,
    /// Grid: the time the columns are scrolled to (a timestamp); 0 shows each column from its first programme.
    grid_top: i64,
    /// Grid: position (in `visible`) of the leftmost column.
    grid_first: usize,
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

/// The narrowest a grid column gets (borders included).
const GRID_MIN_COLUMN: u16 = 28;
/// Characters before the title in a grid entry: favorite dot, time, space.
const GRID_PREFIX: usize = 7;

/// One programme in a grid column: its position in the channel's programmes, and its title wrapped to the column.
struct GridEntry {
    position: usize,
    lines: Vec<String>,
}

/// Breaks text into lines of at most `width` characters, at spaces where possible.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            lines.push(word.chars().take(width).collect());
            word = word.chars().skip(width).collect();
        }
        if current.is_empty() {
            current = word;
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// What a grid column shows: the programmes of `day` that have not ended before `top`, titles wrapped.
fn column_entries(channel: &GuideChannel, day: NaiveDate, top: i64, text_width: usize, ui: &UiOptions) -> Vec<GridEntry> {
    channel
        .on(day)
        .into_iter()
        .filter(|&i| channel.end(i).map_or(true, |end| end.timestamp() > top))
        .map(|i| {
            let p = &channel.programmes[i];
            let mut title = p.title.clone();
            let tags = p.tags_shown(ui.show_quality, ui.show_new, ui.show_rerun);
            if !tags.is_empty() {
                title.push_str(&format!(" [{}]", tags.join(" ")));
            }
            GridEntry { position: i, lines: wrap_text(&title, text_width) }
        })
        .collect()
}

/// The colours of the genre dots of a programme (at most six), from the rules in the settings.
fn genre_dots(ui: &UiOptions, genres: &str) -> Vec<Color> {
    ui.genre_colors_for(genres).into_iter().take(6).map(parse_color).collect()
}

/// Channel positions in the saved order; channels that are not in the saved order follow, as in the file.
fn build_order(guide: &Guide, saved: &[String]) -> Vec<usize> {
    let index: HashMap<&str, usize> =
        guide.channels.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
    let mut used = vec![false; guide.channels.len()];
    let mut order = Vec::with_capacity(guide.channels.len());
    for id in saved {
        if let Some(&i) = index.get(id.as_str()) {
            if !used[i] {
                used[i] = true;
                order.push(i);
            }
        }
    }
    for (i, taken) in used.iter().enumerate() {
        if !taken {
            order.push(i);
        }
    }
    order
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl GuideTab {
    pub fn new(path: PathBuf, order_path: PathBuf) -> Self {
        GuideTab {
            path,
            order: Vec::new(),
            order_path,
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
            grid: false,
            grid_top: 0,
            grid_first: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn guide(&self) -> Option<&Guide> {
        self.guide.as_ref()
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    /// Switches between the list layout and the grid (columns) layout.
    pub fn set_grid(&mut self, grid: bool) {
        if self.grid != grid {
            self.grid = grid;
            self.grid_top = 0;
            if grid && self.focus == Focus::Channels {
                self.focus = Focus::Programmes; // the grid has no separate channel pane
            }
        }
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
    }

    pub fn captures_text(&self) -> bool {
        !matches!(self.mode, Mode::Normal)
    }

    pub fn hints(&self) -> &'static str {
        match &self.mode {
            Mode::Normal if self.grid => {
                "←→ channel · ↑↓ programme · [ ] day · n now · v list · * favorite · K/J move channel · / search · ? help"
            }
            Mode::Normal => {
                "Tab pane · ↑↓ move · ←→ day · n now · v grid · * favorite · K/J move channel · / search · f filter · ? help"
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
                self.order = build_order(&g, &store::read_lines(&self.order_path));
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
        self.visible = self
            .order
            .iter()
            .copied()
            .filter(|&i| q.is_empty() || g.channels[i].search.contains(&q))
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
            KeyCode::Tab | KeyCode::BackTab if self.grid => {
                // the grid has two panes: the columns and the details
                self.focus = if self.focus == Focus::Details { Focus::Programmes } else { Focus::Details };
            }
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
                    _ if self.grid => Focus::Programmes,
                    _ => Focus::Channels,
                }
            }
            KeyCode::Char('v') => return Action::ToggleLayout,
            KeyCode::Char('H') => return self.move_channel(-1),
            KeyCode::Char('L') => return self.move_channel(1),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => return self.move_channel(-1),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => return self.move_channel(1),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => return self.move_channel(-1),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => return self.move_channel(1),
            KeyCode::Char('K') => return self.move_channel(-1),
            KeyCode::Char('J') => return self.move_channel(1),
            KeyCode::Char('*') => {
                if let Some((_, programme)) = self.selected_programme() {
                    return Action::ToggleFavorite(programme.title.clone());
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(-1_000_000),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(1_000_000),
            KeyCode::Left | KeyCode::Char('h') => {
                if self.grid {
                    self.move_column(-1);
                } else {
                    self.move_day(-1);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.grid {
                    self.move_column(1);
                } else {
                    self.move_day(1);
                }
            }
            KeyCode::Char('[') => self.move_day(-1),
            KeyCode::Char(']') => self.move_day(1),
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

    /// Grid: selects the neighbouring channel, on the programme that is on at the same time.
    fn move_column(&mut self, delta: i64) {
        let at = self.selected_programme().map(|(_, p)| p.start.timestamp());
        let before = self.channels.selected();
        step(&mut self.channels, self.visible.len(), delta);
        if self.channels.selected() != before {
            match at {
                Some(ts) => self.select_at_time(ts),
                None => self.select_programme(),
            }
        }
    }

    /// Selects the programme of the selected channel and day that is on at `ts` (a timestamp).
    fn select_at_time(&mut self, ts: i64) {
        self.scroll = 0;
        let list = self.day_list();
        let position = match self.selected_channel() {
            Some(channel) => list.iter().rposition(|&i| channel.programmes[i].start.timestamp() <= ts),
            None => None,
        };
        let mut state = ListState::default();
        if !list.is_empty() {
            state.select(Some(position.unwrap_or(0)));
        }
        self.programmes = state;
    }

    /// Moves the selected channel up or down in the channel list and remembers the order.
    fn move_channel(&mut self, delta: i64) -> Action {
        if !self.grid && self.focus != Focus::Channels {
            return Action::Status("Switch to the channels pane (Tab) to move channels.".to_string());
        }
        if !self.filter.is_empty() {
            return Action::Status("Clear the channel filter (f, then Esc) before moving channels.".to_string());
        }
        let Some(from) = self.channels.selected() else { return Action::None };
        let to = from as i64 + delta;
        if to < 0 || to >= self.order.len() as i64 {
            return Action::None;
        }
        let to = to as usize;

        self.order.swap(from, to);
        self.visible.swap(from, to);
        self.channels.select(Some(to));

        let ids: Vec<String> = match &self.guide {
            Some(g) => self.order.iter().map(|&i| g.channels[i].id.clone()).collect(),
            None => return Action::None,
        };
        match store::write_lines(&self.order_path, &ids) {
            Ok(()) => Action::None,
            Err(e) => Action::Status(format!("Could not save the channel order: {e}")),
        }
    }

    /// Shows the next airing of a show (by normalized title): what is on now or later, else the last one.
    pub fn show_next_airing(&mut self, key: &str) -> bool {
        let target = {
            let Some(g) = &self.guide else { return false };
            let airings = g.airings(key);
            let now = Utc::now().timestamp();
            let upcoming = airings.iter().copied().find(|&(c, p)| {
                g.channels[c].end(p).map_or(true, |end| end.timestamp() > now)
            });
            upcoming.or_else(|| airings.last().copied())
        };
        match target {
            Some((channel, programme)) => {
                self.jump_to(channel, programme);
                true
            }
            None => false,
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
            self.grid_top = 0;
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
    pub fn jump_to(&mut self, channel: usize, programme: usize) {
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

    pub fn draw(&mut self, f: &mut Frame, area: Rect, favs: &Favorites, ui: &UiOptions) {
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
            lines.push(Line::raw("Press 4, then s, to download the guide. Press r to look again."));
            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" Guide ")),
                area,
            );
            return;
        }

        if self.grid {
            let [head, grid, details] =
                Layout::vertical([Constraint::Length(3), Constraint::Percentage(62), Constraint::Min(6)]).areas(area);
            self.draw_header(f, head);
            self.draw_grid(f, grid, favs, ui);
            self.draw_details(f, details);

            if let (Mode::Filter(input), true) = (&self.mode, grid.height >= 3) {
                let popup = centered(grid, grid.width.min(50), 3);
                f.render_widget(Clear, popup);
                let block = Block::bordered().title(" Filter channels ").border_style(Style::new().fg(Color::Yellow));
                let inner = block.inner(popup);
                f.render_widget(block, popup);
                f.render_widget(Paragraph::new(Line::from(input.spans(inner.width as usize))), inner);
            }
            if let (Mode::Search(search), Some(g)) = (&mut self.mode, &self.guide) {
                draw_search(f, area, search, g);
            }
            return;
        }

        let narrow = area.width < 90;
        let [left, right] =
            Layout::horizontal([Constraint::Length(if narrow { 22 } else { 30 }), Constraint::Min(20)]).areas(area);
        let [head, programmes, details] =
            Layout::vertical([Constraint::Length(3), Constraint::Percentage(50), Constraint::Min(4)]).areas(right);

        self.draw_channels(f, left, ui);
        self.draw_header(f, head);
        self.draw_programmes(f, programmes, favs, ui);
        self.draw_details(f, details);

        if let (Mode::Search(search), Some(g)) = (&mut self.mode, &self.guide) {
            draw_search(f, area, search, g);
        }
    }

    fn draw_channels(&mut self, f: &mut Frame, area: Rect, ui: &UiOptions) {
        let (list_area, filter_area) = if matches!(self.mode, Mode::Filter(_)) {
            let [top, bottom] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);
            (top, Some(bottom))
        } else {
            (area, None)
        };

        let code_width = if ui.show_country { 3 } else { 0 };
        let width = list_area.width.saturating_sub(5 + code_width) as usize;
        let (items, total): (Vec<ListItem<'static>>, usize) = match &self.guide {
            Some(g) => (
                self.visible
                    .iter()
                    .map(|&i| {
                        let channel = &g.channels[i];
                        let mut spans = Vec::new();
                        if ui.show_country {
                            spans.push(Span::styled(format!("{:<3}", channel.country), Style::new().fg(Color::Cyan)));
                        }
                        spans.push(Span::raw(fit(&channel.name, width)));
                        ListItem::new(Line::from(spans))
                    })
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

    fn draw_programmes(&mut self, f: &mut Frame, area: Rect, favs: &Favorites, ui: &UiOptions) {
        let focused = self.focus == Focus::Programmes;
        let list = self.day_list();
        let now = Utc::now().timestamp();
        let inner = area.width.saturating_sub(2) as usize;

        let items: Vec<ListItem<'static>> = match self.selected_channel() {
            Some(channel) => list
                .iter()
                .map(|&i| {
                    let p = &channel.programmes[i];
                    let end = channel.end(i).map(|e| e.timestamp());
                    let is_now = p.start.timestamp() <= now && end.is_some_and(|e| now < e);
                    let is_past = end.is_some_and(|e| e <= now);

                    let mut title = p.title.clone();
                    let tags = p.tags_shown(ui.show_quality, ui.show_new, ui.show_rerun);
                    if !tags.is_empty() {
                        title.push_str(&format!("  [{}]", tags.join(" ")));
                    }
                    let style = if is_now {
                        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else if is_past {
                        Style::new().fg(Color::DarkGray)
                    } else {
                        Style::new()
                    };

                    // columns: on-now marker, favorite dot, time, title; the genre dots sit at the right border
                    let favorite = if favs.contains(&p.title) {
                        Span::styled("●", Style::new().fg(parse_color(&ui.favorite_color)))
                    } else {
                        Span::raw(" ")
                    };
                    let dots = genre_dots(ui, &p.genres);
                    let dots_width = dots.len() * 2; // a dot and a space after it
                    let prefix = 9; // marker, favorite dot, " HH:MM "
                    let room = inner.saturating_sub(prefix + dots_width + usize::from(!dots.is_empty()));
                    let shown = fit(&title, room);
                    let pad = inner.saturating_sub(prefix + shown.chars().count() + dots_width);

                    let mut spans = vec![
                        Span::styled(if is_now { ">" } else { " " }, style),
                        favorite,
                        Span::styled(format!(" {} ", p.start.format("%H:%M")), style),
                        Span::styled(shown, style),
                        Span::raw(" ".repeat(pad)),
                    ];
                    for color in dots {
                        spans.push(Span::styled("●", Style::new().fg(color)));
                        spans.push(Span::raw(" "));
                    }
                    ListItem::new(Line::from(spans))
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

    /// The newspaper layout: one column per channel, the day's programmes listed down each column.
    fn draw_grid(&mut self, f: &mut Frame, area: Rect, favs: &Favorites, ui: &UiOptions) {
        let total = self.visible.len();
        if total == 0 {
            f.render_widget(Paragraph::new("No channel matches the filter.").block(Block::bordered()), area);
            return;
        }
        let Some(day) = self.day else {
            f.render_widget(Paragraph::new("The guide has no programmes.").block(Block::bordered()), area);
            return;
        };

        // how many columns fit, and which ones are shown
        let columns = ((area.width / GRID_MIN_COLUMN) as usize).clamp(1, total);
        let selected_column = self.channels.selected().unwrap_or(0).min(total - 1);
        if selected_column < self.grid_first {
            self.grid_first = selected_column;
        } else if selected_column >= self.grid_first + columns {
            self.grid_first = selected_column + 1 - columns;
        }
        self.grid_first = self.grid_first.min(total - columns);

        let rects = Layout::horizontal(vec![Constraint::Ratio(1, columns as u32); columns]).split(area);
        let text_width = (rects[0].width.saturating_sub(2) as usize).saturating_sub(GRID_PREFIX).max(6);
        let inner_height = area.height.saturating_sub(2) as usize;

        // scroll so that the selected programme is on screen, and the columns stay in step with it
        let selected_position = {
            let list = self.day_list();
            self.programmes.selected().and_then(|i| list.get(i).copied())
        };
        if let (Some(g), Some(position), Some(ci)) = (&self.guide, selected_position, self.selected_channel_index()) {
            let channel = &g.channels[ci];
            let mut top = self.grid_top;
            if channel.end(position).is_some_and(|end| end.timestamp() <= top) {
                top = channel.programmes[position].start.timestamp(); // above the top: bring it to the top
            }
            for _ in 0..500 {
                let entries = column_entries(channel, day, top, text_width, ui);
                let mut used = 0usize;
                let mut visible = true;
                for entry in &entries {
                    used += entry.lines.len();
                    if entry.position == position {
                        visible = used <= inner_height || used == entry.lines.len();
                        break;
                    }
                }
                if visible {
                    break;
                }
                // below the bottom: scroll past the first entry
                match entries.first() {
                    Some(first) => {
                        let next = channel.end(first.position).map_or(top + 1, |end| end.timestamp());
                        top = next.max(top + 1);
                    }
                    None => break,
                }
            }
            self.grid_top = top;
        }

        let Some(g) = &self.guide else { return };
        let now = Utc::now().timestamp();
        for (slot, rect) in rects.iter().enumerate() {
            let Some(&ci) = self.visible.get(self.grid_first + slot) else { continue };
            let channel = &g.channels[ci];
            let is_selected = self.grid_first + slot == selected_column;

            let mut lines: Vec<Line<'static>> = Vec::new();
            for entry in column_entries(channel, day, self.grid_top, text_width, ui) {
                if lines.len() >= inner_height {
                    break;
                }
                let p = &channel.programmes[entry.position];
                let end = channel.end(entry.position).map(|e| e.timestamp());
                let is_now = p.start.timestamp() <= now && end.is_some_and(|e| now < e);
                let is_past = end.is_some_and(|e| e <= now);
                let mut style = if is_now {
                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else if is_past {
                    Style::new().fg(Color::DarkGray)
                } else {
                    Style::new()
                };
                if is_selected && Some(entry.position) == selected_position {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                let dots = genre_dots(ui, &p.genres);
                let inner_width = rect.width.saturating_sub(2) as usize;
                for (n, text) in entry.lines.iter().enumerate() {
                    if lines.len() >= inner_height {
                        break;
                    }
                    let mut spans: Vec<Span<'static>> = if n == 0 {
                        let favorite = if favs.contains(&p.title) {
                            Span::styled("●", style.fg(parse_color(&ui.favorite_color)))
                        } else {
                            Span::styled(" ", style)
                        };
                        vec![
                            favorite,
                            Span::styled(p.start.format("%H:%M").to_string(), style),
                            Span::styled(format!(" {text}"), style),
                        ]
                    } else {
                        vec![Span::styled(format!("{}{text}", " ".repeat(GRID_PREFIX)), style)]
                    };

                    // the genre dots go at the right end of the last line of the entry, when they fit there
                    if n + 1 == entry.lines.len() && !dots.is_empty() {
                        let used = GRID_PREFIX + text.chars().count();
                        let dots_width = dots.len() * 2;
                        if used + dots_width <= inner_width {
                            spans.push(Span::styled(" ".repeat(inner_width - used - dots_width), style));
                            for color in &dots {
                                spans.push(Span::styled("●", style.fg(*color)));
                                spans.push(Span::styled(" ", style));
                            }
                        }
                    }
                    lines.push(Line::from(spans));
                }
            }

            let title = if channel.country.is_empty() || !ui.show_country {
                format!(" {} ", channel.name)
            } else {
                format!(" {} {} ", channel.country, channel.name)
            };
            let border = if is_selected && self.focus != Focus::Details { Color::Cyan } else { Color::DarkGray };
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::bordered()
                        .title(title)
                        .title_alignment(Alignment::Center)
                        .border_style(Style::new().fg(border)),
                ),
                *rect,
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_channel_order_comes_first() {
        let xml = r#"<tv>
            <channel id="a"><display-name>A</display-name></channel>
            <channel id="b"><display-name>B</display-name></channel>
            <channel id="c"><display-name>C</display-name></channel>
        </tv>"#;
        let g = guide::parse(xml).unwrap();
        let saved = vec!["c".to_string(), "gone".to_string(), "a".to_string(), "c".to_string()];
        assert_eq!(build_order(&g, &saved), vec![2, 0, 1]);
        assert_eq!(build_order(&g, &[]), vec![0, 1, 2]);
    }

    #[test]
    fn titles_wrap_to_the_column() {
        assert_eq!(wrap_text("Zákon a pořádek: Zločinné úmysly III", 16), vec!["Zákon a pořádek:", "Zločinné úmysly", "III"]);
        assert_eq!(wrap_text("short", 16), vec!["short"]);
        assert_eq!(wrap_text("Dvacetičtyřhodinovka", 8), vec!["Dvacetič", "tyřhodin", "ovka"]);
        assert_eq!(wrap_text("", 8), vec![""]);
    }

    #[test]
    fn grid_columns_start_at_the_scroll_time() {
        let xml = r#"<tv>
            <channel id="cz-a-1"><display-name>A</display-name></channel>
            <programme start="20261029060000 +0100" stop="20261029070000 +0100" channel="cz-a-1"><title>Ranní</title></programme>
            <programme start="20261029070000 +0100" stop="20261029080000 +0100" channel="cz-a-1"><title>Zprávy v osm</title></programme>
            <programme start="20261029080000 +0100" stop="20261029090000 +0100" channel="cz-a-1"><title>Film</title></programme>
        </tv>"#;
        let g = guide::parse(xml).unwrap();
        let ui = UiOptions::default();
        let day = g.dates[0];

        let all = column_entries(&g.channels[0], day, 0, 20, &ui);
        assert_eq!(all.iter().map(|e| e.position).collect::<Vec<_>>(), vec![0, 1, 2]);

        // scrolled to 07:30: the 06:00 programme is gone, the one running at 07:30 is the first
        let top = g.channels[0].programmes[1].start.timestamp() + 30 * 60;
        let later = column_entries(&g.channels[0], day, top, 20, &ui);
        assert_eq!(later.iter().map(|e| e.position).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn selections_stay_inside_their_list() {
        let mut state = ListState::default();
        step(&mut state, 3, 10);
        assert_eq!(state.selected(), Some(2));
        step(&mut state, 3, -10);
        assert_eq!(state.selected(), Some(0));
        step(&mut state, 0, 1);
        assert_eq!(state.selected(), None);
    }
}
