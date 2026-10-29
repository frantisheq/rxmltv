//! Terminal UI (ratatui): a browser for the generated XMLTV guide, and a menu for the updater's settings
//! with a screen that runs the update.

mod guide_tab;
mod input;
mod picker;
mod settings_tab;
mod update_tab;

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

use crate::{config, progress, Options};
use guide_tab::GuideTab;
use settings_tab::SettingsTab;
use update_tab::UpdateTab;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Guide,
    Settings,
    Update,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Tab::Guide => 0,
            Tab::Settings => 1,
            Tab::Update => 2,
        }
    }
}

/// What a tab asks the application to do.
pub enum Action {
    None,
    Status(String),
    Goto(Tab),
    SaveConfig,
    StartUpdate,
    ReloadGuide,
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
    settings: SettingsTab,
    update: UpdateTab,
    help: bool,
    quit: bool,
    quit_warned: bool,
    guide_reload_pending: bool,
    status: Option<(String, Instant)>,
}

/// Runs the terminal UI until the user quits.
pub fn run(opts: Options) -> ExitCode {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!("Error: --tui needs an interactive terminal.");
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
        let config_path = PathBuf::from(opts.config_path.clone().unwrap_or_else(|| config::DEFAULT_PATH.to_string()));
        let guide = GuideTab::new(PathBuf::from(&opts.output));
        App {
            opts,
            config_path,
            handle,
            tab: Tab::Guide,
            guide,
            settings: SettingsTab::new(),
            update: UpdateTab::new(),
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

            if let Some(success) = self.update.poll() {
                if success {
                    self.guide_reload_pending = true;
                    self.set_status("Update finished; the guide is reloaded.");
                }
            }
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
        self.quit = true;
    }

    fn request_quit(&mut self) {
        if self.settings.dirty() && !self.quit_warned {
            self.quit_warned = true;
            self.set_status("Unsaved settings. Press s on the Settings tab to save, or q again to quit anyway.");
        } else {
            self.quit_now();
        }
    }

    fn captures_text(&self) -> bool {
        match self.tab {
            Tab::Guide => self.guide.captures_text(),
            Tab::Settings => self.settings.captures_text(),
            Tab::Update => false,
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
                    self.goto(Tab::Settings);
                    return;
                }
                KeyCode::Char('3') | KeyCode::F(3) => {
                    self.goto(Tab::Update);
                    return;
                }
                _ => {}
            }
        }

        let action = match self.tab {
            Tab::Guide => self.guide.on_key(key),
            Tab::Settings => self.settings.on_key(key, &mut self.opts, &self.handle),
            Tab::Update => self.update.on_key(key),
        };
        self.apply(action);
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Status(text) => self.set_status(text),
            Action::Goto(tab) => self.goto(tab),
            Action::SaveConfig => self.save_config(),
            Action::StartUpdate => {
                self.goto(Tab::Update);
                self.update.start(&self.opts, &self.handle);
            }
            Action::ReloadGuide => self.guide_reload_pending = true,
        }
    }

    fn goto(&mut self, tab: Tab) {
        if tab == Tab::Guide {
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
                let text = format!("Saved the settings to {}", self.config_path.display());
                self.set_status(text);
            }
            Err(e) => {
                let text = format!("Could not save {}: {e}", self.config_path.display());
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
        f.render_widget(
            Paragraph::new(Span::styled(" rxmltv ", Style::new().add_modifier(Modifier::BOLD))),
            title_area,
        );
        f.render_widget(
            Tabs::new(["1 Guide", "2 Settings", "3 Update"])
                .select(self.tab.index())
                .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan))
                .divider(" "),
            tabs_area,
        );

        match self.tab {
            Tab::Guide => self.guide.draw(f, body),
            Tab::Settings => self.settings.draw(f, body, &self.opts, &self.config_path),
            Tab::Update => self.update.draw(f, body, &self.opts),
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
                    Tab::Settings => self.settings.hints(),
                    Tab::Update => UpdateTab::hints(),
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
            Span::styled(format!("{k:<14}"), Style::new().fg(Color::Cyan)),
            Span::raw(text.to_string()),
        ])
    };
    let heading = |text: &str| Line::from(Span::styled(text.to_string(), Style::new().add_modifier(Modifier::BOLD)));

    let lines = vec![
        heading("Everywhere"),
        key("1 2 3 / F1-F3", "Guide, Settings, Update"),
        key("?", "this help"),
        key("q", "quit (Ctrl+C quits at once)"),
        Line::raw(""),
        heading("Guide"),
        key("Tab", "next pane: channels, programmes, details"),
        key("Up Down j k", "move; PgUp PgDn Home End jump"),
        key("Left Right h l", "previous / next day"),
        key("n", "jump to what is on now"),
        key("/", "search programme titles in all channels"),
        key("f", "filter the channel list by name"),
        key("r", "reload the guide file"),
        Line::raw(""),
        heading("Settings"),
        key("Enter / Space", "edit a value, toggle an option, open a list"),
        key("Left Right", "change a number by one"),
        key("s", "save the settings to the settings file"),
        Line::raw(""),
        heading("Update"),
        key("s", "start the update (download the guide)"),
        key("x", "cancel the running update"),
        Line::raw(""),
        Line::styled("Esc or ? closes this help", Style::new().fg(Color::DarkGray)),
    ];

    let popup = centered(area, 64, lines.len() as u16 + 2);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(" Help ").border_style(Style::new().fg(Color::Yellow)))
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
}
