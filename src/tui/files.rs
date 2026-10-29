//! A small file browser, used to choose the SSH key file.

use std::fs;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use super::selected_style;

pub enum Outcome {
    Stay,
    Cancel,
    Chosen(PathBuf),
}

struct Entry {
    name: String,
    is_dir: bool,
}

pub struct FilePicker {
    dir: PathBuf,
    entries: Vec<Entry>,
    state: ListState,
    error: Option<String>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

impl FilePicker {
    /// Starts in the folder of the current choice, else ~/.ssh, else the home folder.
    pub fn open(current: &str) -> Self {
        let from_current = Path::new(current.trim()).parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
        let ssh = home().map(|h| h.join(".ssh")).filter(|p| p.is_dir());
        let dir = from_current.or(ssh).or_else(home).unwrap_or_else(|| PathBuf::from("."));

        let mut picker = FilePicker { dir, entries: Vec::new(), state: ListState::default(), error: None };
        picker.reload();
        picker
    }

    fn reload(&mut self) {
        self.entries.clear();
        self.error = None;
        if self.dir.parent().is_some() {
            self.entries.push(Entry { name: "..".to_string(), is_dir: true });
        }

        match fs::read_dir(&self.dir) {
            Ok(read) => {
                let mut found: Vec<Entry> = read
                    .flatten()
                    .map(|e| Entry {
                        name: e.file_name().to_string_lossy().to_string(),
                        // a symlink to a folder counts as a folder
                        is_dir: e.path().is_dir(),
                    })
                    .collect();
                found.sort_by(|a, b| {
                    b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.entries.extend(found);
            }
            Err(e) => self.error = Some(format!("Cannot read {}: {e}", self.dir.display())),
        }

        // start on the first file, which is usually what is wanted
        let first_file = self.entries.iter().position(|e| !e.is_dir);
        self.state.select(if self.entries.is_empty() { None } else { Some(first_file.unwrap_or(0)) });
    }

    pub fn hints() -> &'static str {
        "↑↓ move · Enter open folder / choose file · Left go up · Esc cancel"
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.dir.parent() {
            self.dir = parent.to_path_buf();
            self.reload();
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        let len = self.entries.len() as i64;
        let current = self.state.selected().unwrap_or(0) as i64;
        let move_to = |to: i64, state: &mut ListState| {
            if len > 0 {
                state.select(Some(to.clamp(0, len - 1) as usize));
            }
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Outcome::Cancel,
            KeyCode::Up | KeyCode::Char('k') => move_to(current - 1, &mut self.state),
            KeyCode::Down | KeyCode::Char('j') => move_to(current + 1, &mut self.state),
            KeyCode::PageUp => move_to(current - 10, &mut self.state),
            KeyCode::PageDown => move_to(current + 10, &mut self.state),
            KeyCode::Home | KeyCode::Char('g') => move_to(0, &mut self.state),
            KeyCode::End | KeyCode::Char('G') => move_to(len, &mut self.state),
            KeyCode::Left | KeyCode::Backspace => self.go_up(),
            KeyCode::Enter | KeyCode::Right => {
                let Some((name, is_dir)) =
                    self.state.selected().and_then(|i| self.entries.get(i)).map(|e| (e.name.clone(), e.is_dir))
                else {
                    return Outcome::Stay;
                };
                if name == ".." {
                    self.go_up();
                } else if is_dir {
                    self.dir = self.dir.join(&name);
                    self.reload();
                } else if key.code == KeyCode::Enter {
                    return Outcome::Chosen(self.dir.join(&name));
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem<'static>> = self
            .entries
            .iter()
            .map(|e| {
                if e.is_dir {
                    ListItem::new(Line::styled(format!("{}/", e.name), Style::new().fg(Color::Cyan)))
                } else {
                    ListItem::new(e.name.clone())
                }
            })
            .collect();

        let title = match &self.error {
            Some(e) => format!(" {e} "),
            None => format!(" {} ", self.dir.display()),
        };
        let list = List::new(items)
            .block(Block::bordered().title(title).border_style(Style::new().fg(Color::Yellow)))
            .highlight_style(selected_style())
            .highlight_symbol("");
        f.render_stateful_widget(list, area, &mut self.state);
    }
}
