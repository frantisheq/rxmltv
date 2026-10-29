//! The "Settings" tab: the updater's options as a menu.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use tokio::runtime::Handle;

use super::input::Input;
use super::picker::Picker;
use super::{centered, selected_style, Action};
use crate::{config, Options};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Days,
    Output,
    CacheDir,
    Concurrency,
    Groups,
    Channels,
    CleanCache,
    StripHdTv,
    KeepTitleNumbers,
    KeepEmptyChannels,
    NoImages,
    RetryMissing,
    RefreshChannels,
    Save,
    Run,
}

const ITEMS: [Item; 15] = [
    Item::Days,
    Item::Output,
    Item::CacheDir,
    Item::Concurrency,
    Item::Groups,
    Item::Channels,
    Item::CleanCache,
    Item::StripHdTv,
    Item::KeepTitleNumbers,
    Item::KeepEmptyChannels,
    Item::NoImages,
    Item::RetryMissing,
    Item::RefreshChannels,
    Item::Save,
    Item::Run,
];

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Days => "Days to download",
            Item::Output => "Output file",
            Item::CacheDir => "Cache folder",
            Item::Concurrency => "Parallel downloads",
            Item::Groups => "Channel groups",
            Item::Channels => "Individual channels",
            Item::CleanCache => "Clean the cache first",
            Item::StripHdTv => "Strip HD / TV from names",
            Item::KeepTitleNumbers => "Keep numbers in titles",
            Item::KeepEmptyChannels => "Write empty channels",
            Item::NoImages => "No programme posters",
            Item::RetryMissing => "Retry missing items",
            Item::RefreshChannels => "Refresh channel list",
            Item::Save => "Save settings",
            Item::Run => "Run the update now",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Item::Days => "How many days to download, starting today. Left/Right change it by one.",
            Item::Output => "Where the XMLTV file is written. A name ending in .gz is compressed. The Guide tab reads this file.",
            Item::CacheDir => "Downloaded files are kept here, so later runs only fetch what is new.",
            Item::Concurrency => "Downloads running at the same time. Be considerate to the server. Left/Right change it by one.",
            Item::Groups => "Enter opens the picker. Pick whole groups (České, Slovenské ...). No groups and no channels means everything.",
            Item::Channels => "Enter opens the picker. Pick single channels, and optionally give them an XMLTV id of your own.",
            Item::CleanCache => "Before updating, delete cached files older than today and leftover temporary files.",
            Item::StripHdTv => "Remove the separate words HD and TV from channel names and ids (Nova HD becomes Nova). Channel ids you picked change with it.",
            Item::KeepTitleNumbers => "Leave (3), (2/6) and roman season numbers in the title instead of writing them as episode numbers.",
            Item::KeepEmptyChannels => "Also write channels that have no programmes.",
            Item::NoImages => "Do not write poster <icon> elements.",
            Item::RetryMissing => "Ask the server again for items it had no data for (normally skipped for 12 hours).",
            Item::RefreshChannels => "Download the channel list again instead of using the cached channels.xml (it is otherwise never refreshed).",
            Item::Save => "Write these settings to the settings file. `rxmltv --config <file>` then uses them without the terminal UI.",
            Item::Run => "Start the update with the settings as they are now (saved or not) and show its progress.",
        }
    }

    fn is_action(self) -> bool {
        matches!(self, Item::Save | Item::Run)
    }

    fn is_flag(self) -> bool {
        matches!(
            self,
            Item::CleanCache
                | Item::StripHdTv
                | Item::KeepTitleNumbers
                | Item::KeepEmptyChannels
                | Item::NoImages
                | Item::RetryMissing
                | Item::RefreshChannels
        )
    }
}

fn flag_ref(item: Item, o: &mut Options) -> Option<&mut bool> {
    match item {
        Item::CleanCache => Some(&mut o.clean_cache),
        Item::StripHdTv => Some(&mut o.strip_hd_tv),
        Item::KeepTitleNumbers => Some(&mut o.keep_title_numbers),
        Item::KeepEmptyChannels => Some(&mut o.keep_empty_channels),
        Item::NoImages => Some(&mut o.no_images),
        Item::RetryMissing => Some(&mut o.retry_missing),
        Item::RefreshChannels => Some(&mut o.refresh_channels),
        _ => None,
    }
}

fn value_text(item: Item, o: &Options) -> String {
    let flag = |on: bool| (if on { "[x]" } else { "[ ]" }).to_string();
    match item {
        Item::Days => o.days.to_string(),
        Item::Output => o.output.clone(),
        Item::CacheDir => o.cache_dir.clone(),
        Item::Concurrency => o.concurrency.to_string(),
        Item::Groups => match &o.groups {
            Some(groups) => groups.join(", "),
            None => "(none picked)".to_string(),
        },
        Item::Channels => match o.channels.len() {
            0 => "(none picked)".to_string(),
            n => format!("{n} picked"),
        },
        Item::CleanCache => flag(o.clean_cache),
        Item::StripHdTv => flag(o.strip_hd_tv),
        Item::KeepTitleNumbers => flag(o.keep_title_numbers),
        Item::KeepEmptyChannels => flag(o.keep_empty_channels),
        Item::NoImages => flag(o.no_images),
        Item::RetryMissing => flag(o.retry_missing),
        Item::RefreshChannels => flag(o.refresh_channels),
        Item::Save | Item::Run => String::new(),
    }
}

struct Edit {
    item: Item,
    input: Input,
    error: Option<String>,
}

pub struct SettingsTab {
    state: ListState,
    edit: Option<Edit>,
    picker: Option<Picker>,
    dirty: bool,
}

/// Checks and applies what was typed for a text or number setting.
fn commit(item: Item, text: &str, o: &mut Options) -> Result<(), String> {
    let text = text.trim();
    match item {
        Item::Days => o.days = crate::parse_positive("Days", text)?,
        Item::Concurrency => o.concurrency = crate::parse_positive("Parallel downloads", text)? as usize,
        Item::Output => {
            if text.is_empty() {
                return Err("The output file name must not be empty.".to_string());
            }
            o.output = text.to_string();
        }
        Item::CacheDir => {
            if text.is_empty() {
                return Err("The cache folder must not be empty.".to_string());
            }
            o.cache_dir = text.to_string();
        }
        _ => {}
    }
    Ok(())
}

impl SettingsTab {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        SettingsTab { state, edit: None, picker: None, dirty: false }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn captures_text(&self) -> bool {
        self.edit.is_some() || self.picker.is_some()
    }

    pub fn hints(&self) -> &'static str {
        if let Some(picker) = &self.picker {
            picker.hints()
        } else if self.edit.is_some() {
            "Enter accept · Esc cancel"
        } else {
            "↑↓ move · Enter edit / toggle / open · ←→ change a number · s save · ? help · q quit"
        }
    }

    /// Called regularly (the channel picker waits for the server in the background).
    pub fn tick(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.tick();
        }
    }

    fn selected(&self) -> Item {
        ITEMS[self.state.selected().unwrap_or(0).min(ITEMS.len() - 1)]
    }

    pub fn on_key(&mut self, key: KeyEvent, o: &mut Options, handle: &Handle) -> Action {
        if let Some(picker) = self.picker.as_mut() {
            let outcome = picker.on_key(key, o);
            if outcome.changed {
                self.dirty = true;
            }
            if outcome.close {
                self.picker = None;
            }
            return Action::None;
        }

        if let Some(edit) = self.edit.as_mut() {
            match key.code {
                KeyCode::Esc => self.edit = None,
                KeyCode::Enter => match commit(edit.item, &edit.input.value(), o) {
                    Ok(()) => {
                        self.edit = None;
                        self.dirty = true;
                    }
                    Err(message) => edit.error = Some(message),
                },
                _ => {
                    let _ = edit.input.handle(&key);
                    edit.error = None;
                }
            }
            return Action::None;
        }

        let last = ITEMS.len() as i64 - 1;
        let current = self.state.selected().unwrap_or(0) as i64;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.state.select(Some((current - 1).clamp(0, last) as usize)),
            KeyCode::Down | KeyCode::Char('j') => self.state.select(Some((current + 1).clamp(0, last) as usize)),
            KeyCode::Home | KeyCode::Char('g') => self.state.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') => self.state.select(Some(last as usize)),
            KeyCode::Char('s') => return Action::SaveConfig,
            KeyCode::Left | KeyCode::Char('-') => self.adjust(o, -1),
            KeyCode::Right | KeyCode::Char('+') => self.adjust(o, 1),
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(o, handle),
            _ => {}
        }
        Action::None
    }

    fn adjust(&mut self, o: &mut Options, delta: i64) {
        match self.selected() {
            Item::Days => o.days = (i64::from(o.days) + delta).clamp(1, 365) as u32,
            Item::Concurrency => o.concurrency = (o.concurrency as i64 + delta).clamp(1, 64) as usize,
            _ => return,
        }
        self.dirty = true;
    }

    fn activate(&mut self, o: &mut Options, handle: &Handle) -> Action {
        let item = self.selected();
        match item {
            Item::Days | Item::Output | Item::CacheDir | Item::Concurrency => {
                let current = match item {
                    Item::Days => o.days.to_string(),
                    Item::Output => o.output.clone(),
                    Item::CacheDir => o.cache_dir.clone(),
                    _ => o.concurrency.to_string(),
                };
                self.edit = Some(Edit { item, input: Input::new(&current), error: None });
                Action::None
            }
            Item::Groups | Item::Channels => {
                self.picker = Some(Picker::open(handle, o.cache_dir.clone(), o.strip_hd_tv));
                Action::None
            }
            Item::Save => Action::SaveConfig,
            Item::Run => Action::StartUpdate,
            _ => {
                if let Some(flag) = flag_ref(item, o) {
                    *flag = !*flag;
                    self.dirty = true;
                }
                if item == Item::StripHdTv && !o.channels.is_empty() {
                    Action::Status(
                        "Channel ids change with this option: check the picked channels again.".to_string(),
                    )
                } else {
                    Action::None
                }
            }
        }
    }

    // ---------- drawing ----------

    pub fn draw(&mut self, f: &mut Frame, area: Rect, o: &Options, config_path: &Path) {
        if let Some(picker) = &mut self.picker {
            picker.draw(f, area, o);
            return;
        }

        let [list_area, info_area] = Layout::vertical([Constraint::Min(8), Constraint::Length(8)]).areas(area);

        let items: Vec<ListItem<'static>> = ITEMS
            .iter()
            .map(|&item| {
                if item.is_action() {
                    let text = if item == Item::Save {
                        format!("▶ {} to {}", item.label(), config_path.display())
                    } else {
                        format!("▶ {}", item.label())
                    };
                    ListItem::new(Line::styled(text, Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)))
                } else {
                    let value_style = if item.is_flag() { Style::new().fg(Color::Yellow) } else { Style::new() };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{:<26}", item.label())),
                        Span::styled(value_text(item, o), value_style),
                    ]))
                }
            })
            .collect();

        let title = if self.dirty { " Updater settings (unsaved changes) " } else { " Updater settings " };
        let list = List::new(items)
            .block(Block::bordered().title(title).border_style(Style::new().fg(Color::Cyan)))
            .highlight_style(selected_style())
            .highlight_symbol("▸ ");
        f.render_stateful_widget(list, list_area, &mut self.state);

        let lines = vec![
            Line::styled(self.selected().help(), Style::new().fg(Color::White)),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Writes: ", Style::new().fg(Color::Cyan)),
                Span::raw(config::selection_summary(o)),
            ]),
            Line::from(vec![
                Span::styled("Command: ", Style::new().fg(Color::Cyan)),
                Span::raw(config::command_line(o)),
            ]),
            Line::from(vec![
                Span::styled("From the saved file: ", Style::new().fg(Color::Cyan)),
                Span::raw(format!("rxmltv --config {}", config_path.display())),
            ]),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" About this setting ")),
            info_area,
        );

        if let Some(edit) = &self.edit {
            let popup = centered(area, area.width.saturating_sub(8).min(80), 6);
            f.render_widget(Clear, popup);
            let block = Block::bordered()
                .title(format!(" {} ", edit.item.label()))
                .border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(popup);
            f.render_widget(block, popup);

            let mut lines = vec![Line::from(edit.input.spans(inner.width.saturating_sub(1) as usize))];
            lines.push(match &edit.error {
                Some(message) => Line::styled(message.clone(), Style::new().fg(Color::Red)),
                None => Line::raw(""),
            });
            lines.push(Line::styled("Enter accepts, Esc cancels", Style::new().fg(Color::DarkGray)));
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_checked() {
        let mut o = Options::default();
        assert!(commit(Item::Days, "3", &mut o).is_ok());
        assert_eq!(o.days, 3);
        assert!(commit(Item::Days, "0", &mut o).is_err());
        assert!(commit(Item::Days, "abc", &mut o).is_err());
        assert_eq!(o.days, 3); // unchanged after a mistake
        assert!(commit(Item::Concurrency, " 8 ", &mut o).is_ok());
        assert_eq!(o.concurrency, 8);
    }

    #[test]
    fn paths_must_not_be_empty() {
        let mut o = Options::default();
        assert!(commit(Item::Output, "   ", &mut o).is_err());
        assert!(commit(Item::Output, "guide.xml.gz", &mut o).is_ok());
        assert_eq!(o.output, "guide.xml.gz");
        assert!(commit(Item::CacheDir, "", &mut o).is_err());
    }

    #[test]
    fn every_flag_has_a_reference() {
        let mut o = Options::default();
        for item in ITEMS {
            assert_eq!(flag_ref(item, &mut o).is_some(), item.is_flag());
        }
    }

    #[test]
    fn flags_show_as_check_boxes() {
        let mut o = Options::default();
        assert_eq!(value_text(Item::NoImages, &o), "[ ]");
        o.no_images = true;
        assert_eq!(value_text(Item::NoImages, &o), "[x]");
    }
}
