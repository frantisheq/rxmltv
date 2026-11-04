//! Terminal UI (ratatui): a browser for the generated XMLTV guide, and a menu for the updater's settings
//! with a screen that runs the update.

mod favorites_tab;
mod files;
mod genres;
mod guide_tab;
mod input;
mod picker;
mod settings_tab;
mod share_tab;
mod update_tab;
mod upload_tab;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Tabs, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tokio::runtime::Handle;

use crate::store::Favorites;
use crate::{config, progress, t, Options};
use favorites_tab::FavoritesTab;
use guide_tab::GuideTab;
use settings_tab::SettingsTab;
use share_tab::ShareTab;
use update_tab::UpdateTab;
use upload_tab::UploadTab;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Guide,
    Favorites,
    Settings,
    Update,
    Upload,
    Share,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Tab::Guide => 0,
            Tab::Favorites => 1,
            Tab::Settings => 2,
            Tab::Update => 3,
            Tab::Upload => 4,
            Tab::Share => 5,
        }
    }
}

/// What a tab asks the application to do.
pub enum Action {
    None,
    Status(String),
    SaveConfig,
    StartUpdate,
    StartUpload,
    /// Delete every cached file (the Settings tab asks for confirmation first).
    PurgeCache,
    /// Switch the Guide between the list and the grid layout.
    ToggleLayout,
    /// Start or stop sharing the output file over HTTP.
    ToggleShare,
    ReloadGuide,
    /// Add the show to the favorites, or remove it when it is one already.
    ToggleFavorite(String),
    AddFavorite(String),
    RemoveFavorite(usize),
    /// Show the next airing of a favorite in the Guide tab.
    ShowFavorite(String),
}

// ---------- small helpers shared by the tabs ----------

/// A rectangle of the given size in the middle of `area` (smaller if `area` is).
pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub(super) fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

/// A colour name (see config::COLOR_NAMES) or #rrggbb. Anything else is white.
pub(super) fn parse_color(text: &str) -> Color {
    let t = text.trim().to_ascii_lowercase();
    match t.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" => Color::Gray,
        "darkgray" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "orange" => Color::Rgb(255, 165, 0),
        "pink" => Color::Rgb(255, 105, 180),
        "purple" => Color::Rgb(160, 32, 240),
        "brown" => Color::Rgb(165, 105, 42),
        "teal" => Color::Rgb(0, 128, 128),
        _ => {
            let hex = t.strip_prefix('#').filter(|h| h.len() == 6);
            match hex.and_then(|h| u32::from_str_radix(h, 16).ok()) {
                Some(v) => Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
                None => Color::White,
            }
        }
    }
}

/// A progress bar as one line of text, with the label in the middle. Drawn by hand (not with a Gauge) so the
/// label is always readable: dark on the filled part, light on the rest.
pub(super) fn progress_line(width: usize, ratio: f64, label: &str) -> Line<'static> {
    let width = width.max(1);
    let mut cells: Vec<char> = vec![' '; width];
    let text: Vec<char> = label.chars().take(width).collect();
    let left = (width - text.len()) / 2;
    for (i, c) in text.iter().enumerate() {
        cells[left + i] = *c;
    }

    let filled = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    let done: String = cells[..filled].iter().collect();
    let rest: String = cells[filled..].iter().collect();
    Line::from(vec![
        Span::styled(done, Style::new().fg(Color::Black).bg(Color::Green)),
        Span::styled(rest, Style::new().fg(Color::White).bg(Color::DarkGray)),
    ])
}

pub(super) fn selected_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

/// Cuts the text to `width` characters, ending in an ellipsis when something was cut off.
pub(super) fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push('…');
    cut
}

// ---------- the application ----------

struct App {
    opts: Options,
    config_path: PathBuf,
    handle: Handle,
    tab: Tab,
    guide: GuideTab,
    favorites: Favorites,
    favorites_tab: FavoritesTab,
    settings: SettingsTab,
    update: UpdateTab,
    upload: UploadTab,
    share: ShareTab,
    help: bool,
    quit: bool,
    quit_warned: bool,
    guide_reload_pending: bool,
    status: Option<(String, Instant)>,
}

/// Runs the terminal UI until the user quits.
pub fn run(opts: Options) -> ExitCode {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!("{}", t!("Error: --tui needs an interactive terminal."));
        return ExitCode::from(2);
    }

    let handle = Handle::current();
    progress::set_quiet(true); // the update's messages go to the Update tab, not to stderr

    let mut app = App::new(opts, handle);
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    progress::set_quiet(false);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(1)
        }
    }
}

impl App {
    fn new(opts: Options, handle: Handle) -> Self {
        let config_path = match &opts.config_path {
            Some(path) => PathBuf::from(path),
            None => config::default_path(),
        };
        let mut guide = GuideTab::new(PathBuf::from(&opts.output), config_path.with_extension("order"));
        guide.set_grid(opts.ui.grid_layout);
        let favorites = Favorites::load(config_path.with_extension("favorites"));
        App {
            opts,
            config_path,
            handle,
            tab: Tab::Guide,
            guide,
            favorites,
            favorites_tab: FavoritesTab::new(),
            settings: SettingsTab::new(),
            update: UpdateTab::new(),
            upload: UploadTab::new(),
            share: ShareTab::new(),
            help: false,
            quit: false,
            quit_warned: false,
            guide_reload_pending: true,
            status: None,
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            if self.guide_reload_pending {
                // Parsing a big guide takes a moment: show "Loading..." first.
                self.guide_reload_pending = false;
                self.guide.set_loading();
                terminal.draw(|f| self.draw(f))?;
                self.guide.reload();
            }

            if let Some(reload) = self.update.poll() {
                if reload {
                    self.guide_reload_pending = true;
                    self.set_status(t!("Update finished; the guide is reloaded."));
                }
            }
            self.upload.poll();
            self.share.sync(&self.opts, &self.handle);
            self.share.poll();
            self.settings.tick();

            terminal.draw(|f| self.draw(f))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key);
                    }
                }
            }
        }
        Ok(())
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn quit_now(&mut self) {
        self.update.shutdown(); // stop a running update and let it remove its temporary files
        self.upload.shutdown();
        self.share.shutdown();
        self.quit = true;
    }

    fn request_quit(&mut self) {
        if self.settings.dirty() && !self.quit_warned {
            self.quit_warned = true;
            self.set_status(t!("Unsaved settings. Press s on the Settings tab to save, or q again to quit anyway."));
        } else {
            self.quit_now();
        }
    }

    fn captures_text(&self) -> bool {
        match self.tab {
            Tab::Guide => self.guide.captures_text(),
            Tab::Favorites => self.favorites_tab.captures_text(),
            Tab::Settings => self.settings.captures_text(),
            Tab::Update | Tab::Upload | Tab::Share => false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit_now();
            return;
        }

        if self.help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q')) {
                self.help = false;
            }
            return;
        }

        if !self.captures_text() {
            match key.code {
                KeyCode::Char('q') => {
                    self.request_quit();
                    return;
                }
                KeyCode::Char('?') => {
                    self.help = true;
                    return;
                }
                KeyCode::Char('1') | KeyCode::F(1) => {
                    self.goto(Tab::Guide);
                    return;
                }
                KeyCode::Char('2') | KeyCode::F(2) => {
                    self.goto(Tab::Favorites);
                    return;
                }
                KeyCode::Char('3') | KeyCode::F(3) => {
                    self.goto(Tab::Settings);
                    return;
                }
                KeyCode::Char('4') | KeyCode::F(4) => {
                    self.goto(Tab::Update);
                    return;
                }
                KeyCode::Char('5') | KeyCode::F(5) => {
                    self.goto(Tab::Upload);
                    return;
                }
                KeyCode::Char('6') | KeyCode::F(6) => {
                    self.goto(Tab::Share);
                    return;
                }
                _ => {}
            }
        }

        let action = match self.tab {
            Tab::Guide => self.guide.on_key(key),
            Tab::Favorites => self.favorites_tab.on_key(key, &self.favorites),
            Tab::Settings => self.settings.on_key(key, &mut self.opts, &self.handle),
            Tab::Update => self.update.on_key(key),
            Tab::Upload => self.upload.on_key(key),
            Tab::Share => self.share.on_key(key),
        };
        self.apply(action);
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Status(text) => self.set_status(text),
            Action::SaveConfig => self.save_config(),
            Action::StartUpdate => {
                self.goto(Tab::Update);
                self.update.start(&self.opts, &self.handle);
            }
            Action::StartUpload => {
                self.goto(Tab::Upload);
                self.upload.start(&self.opts, &self.handle);
            }
            Action::ToggleLayout => {
                self.opts.ui.grid_layout = !self.opts.ui.grid_layout;
                self.guide.set_grid(self.opts.ui.grid_layout);
                self.settings.mark_dirty();
                let name = if self.opts.ui.grid_layout { t!("grid") } else { t!("list") };
                self.set_status(t!("Guide layout: {name} (s on the Settings tab saves it).", name = name));
            }
            Action::ToggleShare => {
                self.opts.tui_serve = !self.opts.tui_serve;
                self.settings.mark_dirty();
            }
            Action::PurgeCache => {
                let text = if self.update.is_running() {
                    t!("An update is running: cancel it (Update tab, x) before purging the cache.").to_string()
                } else {
                    let dir = crate::util::expand_home(&self.opts.cache_dir);
                    match crate::fetcher::purge_cache(&dir) {
                        Ok(report) => t!(
                            "Purged the cache: {files} files, {size}.",
                            files = crate::stats::n0(report.files as u64),
                            size = crate::upload::human_size(report.bytes)
                        ),
                        Err(e) => t!("Could not purge {path}: {error}", path = dir.display(), error = e),
                    }
                };
                self.set_status(text);
            }
            Action::ReloadGuide => self.guide_reload_pending = true,
            Action::ToggleFavorite(title) => {
                let text = match self.favorites.toggle(&title) {
                    Ok(true) => t!("Added \"{title}\" to the favorites.", title = title),
                    Ok(false) => t!("Removed \"{title}\" from the favorites.", title = title),
                    Err(e) => e,
                };
                self.set_status(text);
            }
            Action::AddFavorite(title) => {
                let text = match self.favorites.add(&title) {
                    Ok(()) => t!("\"{title}\" is a favorite.", title = title.trim()),
                    Err(e) => e,
                };
                self.set_status(text);
            }
            Action::RemoveFavorite(index) => {
                if let Err(e) = self.favorites.remove_at(index) {
                    self.set_status(e);
                }
            }
            Action::ShowFavorite(title) => {
                if self.guide.show_next_airing(&crate::store::title_key(&title)) {
                    self.goto(Tab::Guide);
                } else {
                    self.set_status(t!("\"{title}\" is not in the guide.", title = title));
                }
            }
        }
    }

    fn goto(&mut self, tab: Tab) {
        if tab == Tab::Guide {
            self.guide.set_grid(self.opts.ui.grid_layout); // it may have been changed on the Settings tab
            // The output file may have been changed on the Settings tab.
            let path = PathBuf::from(&self.opts.output);
            if self.guide.path() != path.as_path() {
                self.guide.set_path(path);
                self.guide_reload_pending = true;
            }
        }
        self.tab = tab;
    }

    fn save_config(&mut self) {
        match config::save(&self.config_path, &self.opts) {
            Ok(()) => {
                self.settings.mark_saved();
                self.quit_warned = false;
                let text = t!("Saved the settings to {path}", path = self.config_path.display());
                self.set_status(text);
            }
            Err(e) => {
                let text = t!("Could not save {path}: {error}", path = self.config_path.display(), error = e);
                self.set_status(text);
            }
        }
    }

    // ---------- drawing ----------

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);

        let [title_area, tabs_area] = Layout::horizontal([Constraint::Length(8), Constraint::Min(10)]).areas(header);
        let tab_titles = [
            format!("1 {}", t!("Guide")),
            format!("2 {}", t!("Favorites")),
            format!("3 {}", t!("Settings")),
            format!("4 {}", t!("Update")),
            format!("5 {}", t!("Upload")),
            format!("6 {}", t!("Share")),
        ];
        f.render_widget(
            Paragraph::new(Span::styled(" rxmltv ", Style::new().add_modifier(Modifier::BOLD))),
            title_area,
        );
        f.render_widget(
            Tabs::new(tab_titles)
                .select(self.tab.index())
                .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan))
                .divider(" "),
            tabs_area,
        );

        match self.tab {
            Tab::Guide => self.guide.draw(f, body, &self.favorites, &self.opts.ui),
            Tab::Favorites => {
                self.favorites_tab.draw(f, body, &self.favorites, self.guide.guide(), &self.opts.ui)
            }
            Tab::Settings => {
                let categories = self.guide.guide().map(|g| g.top_categories.as_slice()).unwrap_or(&[]);
                self.settings.draw(f, body, &self.opts, &self.config_path, categories)
            }
            Tab::Update => self.update.draw(f, body, &self.opts),
            Tab::Upload => self.upload.draw(f, body, &self.opts),
            Tab::Share => self.share.draw(f, body, &self.opts),
        }

        self.draw_footer(f, footer);

        if self.help {
            draw_help(f, area);
        }
    }

    fn draw_footer(&mut self, f: &mut Frame, area: Rect) {
        if self.status.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(6)) {
            self.status = None;
        }

        let line = match &self.status {
            Some((text, _)) => Line::from(Span::styled(text.clone(), Style::new().fg(Color::Yellow))),
            None => {
                let hints = match self.tab {
                    Tab::Guide => self.guide.hints(),
                    Tab::Favorites => self.favorites_tab.hints(),
                    Tab::Settings => self.settings.hints(),
                    Tab::Update => UpdateTab::hints(),
                    Tab::Upload => UploadTab::hints(),
                    Tab::Share => ShareTab::hints(),
                };
                Line::from(Span::styled(hints, Style::new().fg(Color::DarkGray)))
            }
        };
        f.render_widget(Paragraph::new(line), area);
    }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let key = |k: &str, text: &str| {
        Line::from(vec![
            Span::styled(format!("{k:<22}"), Style::new().fg(Color::Cyan)),
            Span::raw(text.to_string()),
        ])
    };
    let heading = |text: &str| Line::from(Span::styled(text.to_string(), Style::new().add_modifier(Modifier::BOLD)));

    let lines = vec![
        heading(t!("Everywhere")),
        key("1-6 / F1-F6", t!("Guide, Favorites, Settings, Update, Upload, Share")),
        key("?", t!("this help")),
        key("q", t!("quit (Ctrl+C quits at once)")),
        Line::raw(""),
        heading(t!("Guide")),
        key("Tab", t!("next pane: channels, programmes, details")),
        key("Up Down j k", t!("move; PgUp PgDn Home End jump")),
        key("Left Right h l", t!("previous / next day")),
        key("n", t!("jump to what is on now")),
        key("p", t!("jump to primetime")),
        key("v", t!("switch between the list and the grid layout")),
        key("[ ]", t!("previous / next day (in the grid, ← → pick the channel)")),
        key("/", t!("search programme titles in all channels")),
        key("f", t!("filter the channel list by name")),
        key("r", t!("reload the guide file")),
        key("*", t!("add / remove the show as a favorite")),
        key("K J / Shift+Up Down", t!("move the channel up / down (channels pane)")),
        Line::raw(""),
        heading(t!("Favorites")),
        key("Enter", t!("show the next airing in the Guide")),
        key("a  d", t!("add a title / remove the selected one")),
        Line::raw(""),
        heading(t!("Settings")),
        key("Enter / Space", t!("edit a value, toggle an option, open a list")),
        key("Left Right", t!("change a number, or the protocol, by one")),
        key("s", t!("save the settings to the settings file")),
        Line::raw(""),
        heading(t!("Update")),
        key("s", t!("start the update (download the guide)")),
        key("x", t!("cancel the running update")),
        Line::raw(""),
        heading(t!("Upload")),
        key("s", t!("upload the output file now")),
        key("x", t!("cancel the upload")),
        Line::raw(""),
        heading(t!("Share")),
        key("s", t!("start / stop sharing over HTTP")),
        Line::raw(""),
        Line::styled(t!("Esc or ? closes this help"), Style::new().fg(Color::DarkGray)),
    ];

    let popup = centered(area, 84, lines.len() as u16 + 2);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" {} ", t!("Help"))).border_style(Style::new().fg(Color::Yellow)))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        assert_eq!(fit("Nova Cinema", 20), "Nova Cinema");
        assert_eq!(fit("Nova Cinema", 5), "Nova…");
        assert_eq!(fit("Nova", 0), "");
        assert_eq!(fit("Česká televize", 6), "Česká…");
    }

    #[test]
    fn popups_never_exceed_the_screen() {
        let area = Rect::new(0, 0, 40, 10);
        let r = centered(area, 100, 100);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 40, 10));
        let r = centered(area, 20, 4);
        assert_eq!((r.x, r.y, r.width, r.height), (10, 3, 20, 4));
    }

    #[test]
    fn colours_are_parsed() {
        assert_eq!(parse_color("Red"), Color::Red);
        assert_eq!(parse_color("#ff8000"), Color::Rgb(255, 128, 0));
        assert_eq!(parse_color("nonsense"), Color::White);
        assert_eq!(parse_color("orange"), Color::Rgb(255, 165, 0));
    }

    #[test]
    fn the_progress_bar_keeps_its_width_and_label() {
        let line = progress_line(20, 0.5, "10/20 channels");
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text.chars().count(), 20);
        assert!(text.contains("10/20 channels"));
        // the label crosses the boundary between the two colours, and stays readable on both sides
        assert_eq!(line.spans[0].style.fg, Some(Color::Black));
        assert_eq!(line.spans[1].style.fg, Some(Color::White));

        let empty = progress_line(10, 0.0, "");
        assert_eq!(empty.spans[0].content.chars().count(), 0);
        let full = progress_line(10, 2.0, "x"); // out of range is clamped
        assert_eq!(full.spans[0].content.chars().count(), 10);
        assert_eq!(progress_line(3, 0.5, "far too long").width(), 3);
    }
}
