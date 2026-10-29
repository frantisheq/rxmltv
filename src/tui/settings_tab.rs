//! The "Settings" tab: the updater's options as a menu.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use tokio::runtime::Handle;

use super::files::{self, FilePicker};
use super::genres::GenreEditor;
use super::input::Input;
use super::picker::{Kind, Picker};
use super::{centered, parse_color, selected_style, Action};
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
    CompressCache,
    StripHdTv,
    KeepTitleNumbers,
    KeepEmptyChannels,
    NoImages,
    RetryMissing,
    RefreshChannels,
    GuideLayout,
    ShowQuality,
    ShowNew,
    ShowRerun,
    FavoriteColor,
    GenreColors,
    PostCommand,
    UploadAuto,
    UploadProtocol,
    UploadHost,
    UploadPort,
    UploadUser,
    UploadPassword,
    UploadFolder,
    UploadName,
    UploadKey,
    UploadPassphrase,
    UploadInsecure,
    Save,
    Run,
    UploadNow,
    PurgeCache,
}

const ITEMS: [Item; 36] = [
    Item::Days,
    Item::Output,
    Item::CacheDir,
    Item::Concurrency,
    Item::Groups,
    Item::Channels,
    Item::CleanCache,
    Item::CompressCache,
    Item::StripHdTv,
    Item::KeepTitleNumbers,
    Item::KeepEmptyChannels,
    Item::NoImages,
    Item::RetryMissing,
    Item::RefreshChannels,
    Item::GuideLayout,
    Item::ShowQuality,
    Item::ShowNew,
    Item::ShowRerun,
    Item::FavoriteColor,
    Item::GenreColors,
    Item::PostCommand,
    Item::UploadAuto,
    Item::UploadProtocol,
    Item::UploadHost,
    Item::UploadPort,
    Item::UploadUser,
    Item::UploadPassword,
    Item::UploadFolder,
    Item::UploadName,
    Item::UploadKey,
    Item::UploadPassphrase,
    Item::UploadInsecure,
    Item::Save,
    Item::Run,
    Item::UploadNow,
    Item::PurgeCache,
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
            Item::CompressCache => "Compress cache files",
            Item::StripHdTv => "Strip HD / TV from names",
            Item::KeepTitleNumbers => "Keep numbers in titles",
            Item::KeepEmptyChannels => "Write empty channels",
            Item::NoImages => "No programme posters",
            Item::RetryMissing => "Retry missing items",
            Item::RefreshChannels => "Refresh channel list",
            Item::GuideLayout => "Guide layout",
            Item::ShowQuality => "Show quality tag (HDTV)",
            Item::ShowNew => "Show NEW tag",
            Item::ShowRerun => "Show RERUN tag",
            Item::FavoriteColor => "Favorite dot colour",
            Item::GenreColors => "Genre dot colours",
            Item::PostCommand => "Command after update",
            Item::UploadAuto => "Upload after update",
            Item::UploadProtocol => "Upload: protocol",
            Item::UploadHost => "Upload: server",
            Item::UploadPort => "Upload: port",
            Item::UploadUser => "Upload: user name",
            Item::UploadPassword => "Upload: password",
            Item::UploadFolder => "Upload: folder",
            Item::UploadName => "Upload: file name",
            Item::UploadKey => "Upload: SSH key file",
            Item::UploadPassphrase => "Upload: key passphrase",
            Item::UploadInsecure => "Skip certificate check",
            Item::Save => "Save settings",
            Item::Run => "Run the update now",
            Item::UploadNow => "Upload now",
            Item::PurgeCache => "Purge the cache now",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Item::Days => "How many days to download, starting today. Left/Right change it by one.",
            Item::Output => "Where the XMLTV file is written. A name ending in .gz is compressed. The Guide tab reads this file.",
            Item::CacheDir => "Downloaded files are kept here, so later runs only fetch what is new.",
            Item::Concurrency => "Downloads running at the same time. Be considerate to the server. Left/Right change it by one.",
            Item::Groups => "Enter opens the group picker. No groups and no channels picked means every channel.",
            Item::Channels => "Enter opens the channel picker: pick single channels, and optionally give them an XMLTV id of your own.",
            Item::CleanCache => "Before updating, delete cached files older than today and leftover temporary files.",
            Item::CompressCache => "Store cache files gzip-compressed to save disk space (usually about a fifth of the size). Existing files are compressed at the next update; both kinds are always readable.",
            Item::StripHdTv => "Remove the separate words HD and TV from channel names and ids (Nova HD becomes Nova). Channel ids you picked change with it.",
            Item::KeepTitleNumbers => "Leave (3), (2/6) and roman season numbers in the title instead of writing them as episode numbers.",
            Item::KeepEmptyChannels => "Also write channels that have no programmes.",
            Item::NoImages => "Do not write poster <icon> elements.",
            Item::RetryMissing => "Ask the server again for items it had no data for (normally skipped for 12 hours).",
            Item::RefreshChannels => "Download the channel list again instead of using the cached channels.xml (it is otherwise never refreshed).",
            Item::GuideLayout => "list: channel list and one programme list. grid: one column per channel, like a newspaper listing, with the details below. Enter or Left/Right switch; v switches in the Guide.",
            Item::ShowQuality => "Show the quality tag (HDTV) after the show name in the Guide's programme list.",
            Item::ShowNew => "Show the NEW tag after the show name in the Guide's programme list.",
            Item::ShowRerun => "Show the RERUN tag after the show name in the Guide's programme list.",
            Item::FavoriteColor => "The colour of the dot before the time of a favorite show. A colour name (red, orange ...) or #rrggbb.",
            Item::GenreColors => "Enter opens the editor: which genre gets which dot colour before the show name.",
            Item::PostCommand => "A shell command run after a successful update, before the upload. {file} is the output file (also in $RXMLTV_OUTPUT). Empty: none.",
            Item::UploadAuto => "Upload the output file to the server below after every update. The Upload tab can always upload by hand. Needs the curl program.",
            Item::UploadProtocol => "ftp, ftps (FTP with TLS), sftp (over SSH), webdav (http) or webdavs (https). Enter or Left/Right change it.",
            Item::UploadHost => "The server's name or address.",
            Item::UploadPort => "Empty: the protocol's usual port (21 FTP/FTPS, 22 SFTP, 80 WebDAV, 443 WebDAVs).",
            Item::UploadUser => "The user name to log in with. Empty: none.",
            Item::UploadPassword => "Saved as plain text in the settings file (readable only by you on Linux/macOS). Empty: $RXMLTV_UPLOAD_PASSWORD, if set.",
            Item::UploadFolder => "The folder on the server. A leading / is an absolute path (FTP, SFTP); otherwise it is relative to the login folder. Folders are created (FTP, SFTP). WebDAV folders must exist.",
            Item::UploadName => "The file name on the server. Empty: the name of the output file.",
            Item::UploadKey => "SFTP: the SSH private key to log in with. Enter browses for the file, e types the path. Empty: log in with the password.",
            Item::UploadPassphrase => "SFTP: the passphrase of the SSH key, if it has one. Saved as plain text in the settings file.",
            Item::UploadInsecure => "Do not verify the TLS certificate (FTPS, https) or the SSH host key (SFTP). Only for servers you trust, e.g. a self-signed certificate.",
            Item::Save => "Write these settings to the settings file. `rxmltv --config <file>` then uses them without the terminal UI.",
            Item::Run => "Start the update with the settings as they are now (saved or not), then the command and upload if set.",
            Item::UploadNow => "Upload the output file now, with a progress bar on the Upload tab.",
            Item::PurgeCache => "Delete every cached file, including the channel list (press Enter twice). The next update downloads everything again.",
        }
    }

    fn is_action(self) -> bool {
        matches!(self, Item::Save | Item::Run | Item::UploadNow | Item::PurgeCache)
    }

    fn is_flag(self) -> bool {
        matches!(
            self,
            Item::CleanCache
                | Item::CompressCache
                | Item::StripHdTv
                | Item::KeepTitleNumbers
                | Item::KeepEmptyChannels
                | Item::NoImages
                | Item::RetryMissing
                | Item::RefreshChannels
                | Item::ShowQuality
                | Item::ShowNew
                | Item::ShowRerun
                | Item::UploadAuto
                | Item::UploadInsecure
        )
    }

    /// Typed into a one-line field.
    fn is_text(self) -> bool {
        matches!(
            self,
            Item::Days
                | Item::Output
                | Item::CacheDir
                | Item::Concurrency
                | Item::FavoriteColor
                | Item::PostCommand
                | Item::UploadHost
                | Item::UploadPort
                | Item::UploadUser
                | Item::UploadPassword
                | Item::UploadFolder
                | Item::UploadName
                | Item::UploadKey
                | Item::UploadPassphrase
        )
    }

    fn is_secret(self) -> bool {
        matches!(self, Item::UploadPassword | Item::UploadPassphrase)
    }
}

fn flag_ref(item: Item, o: &mut Options) -> Option<&mut bool> {
    match item {
        Item::CleanCache => Some(&mut o.clean_cache),
        Item::CompressCache => Some(&mut o.compress_cache),
        Item::StripHdTv => Some(&mut o.strip_hd_tv),
        Item::KeepTitleNumbers => Some(&mut o.keep_title_numbers),
        Item::KeepEmptyChannels => Some(&mut o.keep_empty_channels),
        Item::NoImages => Some(&mut o.no_images),
        Item::RetryMissing => Some(&mut o.retry_missing),
        Item::RefreshChannels => Some(&mut o.refresh_channels),
        Item::ShowQuality => Some(&mut o.ui.show_quality),
        Item::ShowNew => Some(&mut o.ui.show_new),
        Item::ShowRerun => Some(&mut o.ui.show_rerun),
        Item::UploadAuto => Some(&mut o.upload.enabled),
        Item::UploadInsecure => Some(&mut o.upload.insecure),
        _ => None,
    }
}

fn flag_get(item: Item, o: &Options) -> bool {
    match item {
        Item::CleanCache => o.clean_cache,
        Item::CompressCache => o.compress_cache,
        Item::StripHdTv => o.strip_hd_tv,
        Item::KeepTitleNumbers => o.keep_title_numbers,
        Item::KeepEmptyChannels => o.keep_empty_channels,
        Item::NoImages => o.no_images,
        Item::RetryMissing => o.retry_missing,
        Item::RefreshChannels => o.refresh_channels,
        Item::ShowQuality => o.ui.show_quality,
        Item::ShowNew => o.ui.show_new,
        Item::ShowRerun => o.ui.show_rerun,
        Item::UploadAuto => o.upload.enabled,
        Item::UploadInsecure => o.upload.insecure,
        _ => false,
    }
}

/// The current value of a typed setting, as it is edited.
fn text_of(item: Item, o: &Options) -> String {
    match item {
        Item::Days => o.days.to_string(),
        Item::Output => o.output.clone(),
        Item::CacheDir => o.cache_dir.clone(),
        Item::Concurrency => o.concurrency.to_string(),
        Item::FavoriteColor => o.ui.favorite_color.clone(),
        Item::PostCommand => o.post_command.clone(),
        Item::UploadHost => o.upload.host.clone(),
        Item::UploadPort => o.upload.port.map(|p| p.to_string()).unwrap_or_default(),
        Item::UploadUser => o.upload.username.clone(),
        Item::UploadPassword => o.upload.password.clone(),
        Item::UploadFolder => o.upload.folder.clone(),
        Item::UploadName => o.upload.remote_name.clone(),
        Item::UploadKey => o.upload.key_file.clone(),
        Item::UploadPassphrase => o.upload.key_passphrase.clone(),
        _ => String::new(),
    }
}

/// What the menu shows after the label.
fn value_text(item: Item, o: &Options) -> String {
    let flag = |on: bool| (if on { "[x]" } else { "[ ]" }).to_string();
    let or_unset = |text: String| if text.is_empty() { "(not set)".to_string() } else { text };
    match item {
        Item::Groups => match &o.groups {
            Some(groups) => groups.join(", "),
            None => "(none picked)".to_string(),
        },
        Item::Channels => match o.channels.len() {
            0 => "(none picked)".to_string(),
            n => format!("{n} picked"),
        },
        Item::GenreColors => format!("{} rules", o.ui.genre_colors.len()),
        Item::GuideLayout => (if o.ui.grid_layout { "grid (channels as columns)" } else { "list" }).to_string(),
        Item::UploadProtocol => o.upload.protocol.label().to_string(),
        Item::UploadPort => match o.upload.port {
            Some(port) => port.to_string(),
            None => format!("{} (usual)", o.upload.protocol.default_port()),
        },
        Item::UploadPassword | Item::UploadPassphrase => {
            if text_of(item, o).is_empty() {
                "(not set)".to_string()
            } else {
                "********".to_string()
            }
        }
        Item::PostCommand | Item::UploadHost | Item::UploadUser | Item::UploadFolder | Item::UploadKey => {
            or_unset(text_of(item, o))
        }
        Item::UploadName => {
            let own = text_of(item, o);
            if own.is_empty() {
                "(output file name)".to_string()
            } else {
                own
            }
        }
        Item::Days | Item::Output | Item::CacheDir | Item::Concurrency | Item::FavoriteColor => text_of(item, o),
        Item::CleanCache
        | Item::CompressCache
        | Item::StripHdTv
        | Item::KeepTitleNumbers
        | Item::KeepEmptyChannels
        | Item::NoImages
        | Item::RetryMissing
        | Item::RefreshChannels
        | Item::ShowQuality
        | Item::ShowNew
        | Item::ShowRerun
        | Item::UploadAuto
        | Item::UploadInsecure => flag(flag_get(item, o)),
        Item::Save | Item::Run | Item::UploadNow | Item::PurgeCache => String::new(),
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
    genres: Option<GenreEditor>,
    files: Option<FilePicker>,
    dirty: bool,
    /// Enter was pressed once on "Purge the cache now"; the next Enter does it.
    purge_armed: bool,
}

/// Checks and applies what was typed for a setting.
fn commit(item: Item, text: &str, o: &mut Options) -> Result<(), String> {
    let text = text.trim();
    let not_empty = |what: &str| {
        if text.is_empty() {
            Err(format!("{what} must not be empty."))
        } else {
            Ok(text.to_string())
        }
    };
    match item {
        Item::Days => o.days = crate::parse_positive("Days", text)?,
        Item::Concurrency => o.concurrency = crate::parse_positive("Parallel downloads", text)? as usize,
        Item::Output => o.output = not_empty("The output file name")?,
        Item::CacheDir => o.cache_dir = not_empty("The cache folder")?,
        Item::FavoriteColor => {
            if !config::valid_color(text) {
                return Err("Not a colour: use a name like red, orange, lightblue, or #rrggbb.".to_string());
            }
            o.ui.favorite_color = text.to_ascii_lowercase();
        }
        Item::PostCommand => o.post_command = text.to_string(),
        Item::UploadHost => o.upload.host = text.to_string(),
        Item::UploadPort => {
            o.upload.port = if text.is_empty() {
                None
            } else {
                Some(
                    text.parse::<u16>()
                        .ok()
                        .filter(|p| *p > 0)
                        .ok_or_else(|| "The port must be a number from 1 to 65535.".to_string())?,
                )
            };
        }
        Item::UploadUser => o.upload.username = text.to_string(),
        Item::UploadPassword => o.upload.password = text.to_string(),
        Item::UploadFolder => o.upload.folder = text.to_string(),
        Item::UploadName => o.upload.remote_name = text.to_string(),
        Item::UploadKey => o.upload.key_file = text.to_string(),
        Item::UploadPassphrase => o.upload.key_passphrase = text.to_string(),
        _ => {}
    }
    Ok(())
}

impl SettingsTab {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        SettingsTab { state, edit: None, picker: None, genres: None, files: None, dirty: false, purge_armed: false }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn captures_text(&self) -> bool {
        self.edit.is_some() || self.picker.is_some() || self.genres.is_some() || self.files.is_some()
    }

    pub fn hints(&self) -> &'static str {
        if let Some(picker) = &self.picker {
            picker.hints()
        } else if let Some(genres) = &self.genres {
            genres.hints()
        } else if self.files.is_some() {
            FilePicker::hints()
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

        if let Some(genres) = self.genres.as_mut() {
            let outcome = genres.on_key(key, o);
            if outcome.changed {
                self.dirty = true;
            }
            if outcome.close {
                self.genres = None;
            }
            return Action::None;
        }

        if let Some(picker) = self.files.as_mut() {
            match picker.on_key(key) {
                files::Outcome::Stay => {}
                files::Outcome::Cancel => self.files = None,
                files::Outcome::Chosen(path) => {
                    o.upload.key_file = path.to_string_lossy().to_string();
                    self.dirty = true;
                    self.files = None;
                }
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

        let armed = std::mem::take(&mut self.purge_armed); // any other key cancels it
        let last = ITEMS.len() as i64 - 1;
        let current = self.state.selected().unwrap_or(0) as i64;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.state.select(Some((current - 1).clamp(0, last) as usize)),
            KeyCode::Down | KeyCode::Char('j') => self.state.select(Some((current + 1).clamp(0, last) as usize)),
            KeyCode::PageUp => self.state.select(Some((current - 10).clamp(0, last) as usize)),
            KeyCode::PageDown => self.state.select(Some((current + 10).clamp(0, last) as usize)),
            KeyCode::Home | KeyCode::Char('g') => self.state.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') => self.state.select(Some(last as usize)),
            KeyCode::Char('s') => return Action::SaveConfig,
            KeyCode::Char('e') if self.selected() == Item::UploadKey => self.begin_edit(o),
            KeyCode::Left | KeyCode::Char('-') => self.adjust(o, -1),
            KeyCode::Right | KeyCode::Char('+') => self.adjust(o, 1),
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(o, handle, armed),
            _ => {}
        }
        Action::None
    }

    fn adjust(&mut self, o: &mut Options, delta: i64) {
        match self.selected() {
            Item::Days => o.days = (i64::from(o.days) + delta).clamp(1, 365) as u32,
            Item::Concurrency => o.concurrency = (o.concurrency as i64 + delta).clamp(1, 64) as usize,
            Item::UploadProtocol => o.upload.protocol = o.upload.protocol.step(delta as i32),
            Item::GuideLayout => o.ui.grid_layout = !o.ui.grid_layout,
            _ => return,
        }
        self.dirty = true;
    }

    fn begin_edit(&mut self, o: &Options) {
        let item = self.selected();
        self.edit = Some(Edit { item, input: Input::new(&text_of(item, o)), error: None });
    }

    fn activate(&mut self, o: &mut Options, handle: &Handle, armed: bool) -> Action {
        let item = self.selected();
        match item {
            Item::Groups | Item::Channels => {
                let kind = if item == Item::Groups { Kind::Groups } else { Kind::Channels };
                self.picker = Some(Picker::open(handle, o.cache_dir.clone(), o.strip_hd_tv, kind));
                Action::None
            }
            Item::GenreColors => {
                self.genres = Some(GenreEditor::open(o));
                Action::None
            }
            Item::UploadKey => {
                self.files = Some(FilePicker::open(&o.upload.key_file));
                Action::None
            }
            Item::UploadProtocol | Item::GuideLayout => {
                self.adjust(o, 1);
                Action::None
            }
            Item::Save => Action::SaveConfig,
            Item::Run => Action::StartUpdate,
            Item::UploadNow => Action::StartUpload,
            Item::PurgeCache => {
                if armed {
                    Action::PurgeCache
                } else {
                    self.purge_armed = true;
                    Action::Status(format!(
                        "This deletes ALL cached files in {}. Press Enter again to do it, any other key to cancel.",
                        crate::util::expand_home(&o.cache_dir).display()
                    ))
                }
            }
            _ if item.is_text() => {
                self.begin_edit(o);
                Action::None
            }
            _ => {
                if let Some(flag) = flag_ref(item, o) {
                    *flag = !*flag;
                    self.dirty = true;
                }
                if item == Item::StripHdTv && !o.channels.is_empty() {
                    Action::Status("Channel ids change with this option: check the picked channels again.".to_string())
                } else {
                    Action::None
                }
            }
        }
    }

    // ---------- drawing ----------

    pub fn draw(
        &mut self,
        f: &mut Frame,
        area: Rect,
        o: &Options,
        config_path: &Path,
        categories: &[(String, usize)],
    ) {
        if let Some(picker) = &mut self.picker {
            picker.draw(f, area, o);
            return;
        }
        if let Some(genres) = &mut self.genres {
            genres.draw(f, area, o, categories);
            return;
        }
        if let Some(picker) = &mut self.files {
            picker.draw(f, area);
            return;
        }

        let [list_area, info_area] = Layout::vertical([Constraint::Min(8), Constraint::Length(9)]).areas(area);

        let items: Vec<ListItem<'static>> = ITEMS
            .iter()
            .map(|&item| {
                if item.is_action() {
                    let text = match item {
                        Item::Save => format!("> {} to {}", item.label(), config_path.display()),
                        _ => format!("> {}", item.label()),
                    };
                    ListItem::new(Line::styled(text, Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)))
                } else {
                    let mut spans = vec![Span::raw(format!("{:<26}", item.label()))];
                    if item == Item::FavoriteColor {
                        spans.push(Span::styled("● ", Style::new().fg(parse_color(&o.ui.favorite_color))));
                    }
                    let value_style = if item.is_flag() { Style::new().fg(Color::Yellow) } else { Style::new() };
                    spans.push(Span::styled(value_text(item, o), value_style));
                    ListItem::new(Line::from(spans))
                }
            })
            .collect();

        let title = if self.dirty { " Settings (unsaved changes) " } else { " Settings " };
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

            let width = inner.width.saturating_sub(1) as usize;
            let spans = if edit.item.is_secret() { edit.input.masked_spans(width) } else { edit.input.spans(width) };
            let mut lines = vec![Line::from(spans)];
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
    fn ports_and_colours_are_checked() {
        let mut o = Options::default();
        assert!(commit(Item::UploadPort, "2121", &mut o).is_ok());
        assert_eq!(o.upload.port, Some(2121));
        assert!(commit(Item::UploadPort, "0", &mut o).is_err());
        assert!(commit(Item::UploadPort, "99999", &mut o).is_err());
        assert_eq!(o.upload.port, Some(2121));
        assert!(commit(Item::UploadPort, "", &mut o).is_ok());
        assert_eq!(o.upload.port, None);

        assert!(commit(Item::FavoriteColor, "Pink", &mut o).is_ok());
        assert_eq!(o.ui.favorite_color, "pink");
        assert!(commit(Item::FavoriteColor, "no", &mut o).is_err());
        assert_eq!(o.ui.favorite_color, "pink");
    }

    #[test]
    fn upload_texts_are_stored() {
        let mut o = Options::default();
        commit(Item::UploadHost, " ftp.example.com ", &mut o).unwrap();
        commit(Item::UploadPassword, "secret", &mut o).unwrap();
        commit(Item::PostCommand, "gzip -k {file}", &mut o).unwrap();
        assert_eq!(o.upload.host, "ftp.example.com");
        assert_eq!(o.upload.password, "secret");
        assert_eq!(o.post_command, "gzip -k {file}");
        assert_eq!(value_text(Item::UploadPassword, &o), "********"); // never shown
        assert_eq!(text_of(Item::UploadHost, &o), "ftp.example.com");
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
        assert_eq!(value_text(Item::ShowRerun, &o), "[x]"); // on by default
    }

    #[test]
    fn the_item_list_is_complete() {
        // every item appears exactly once
        for (i, a) in ITEMS.iter().enumerate() {
            for b in &ITEMS[i + 1..] {
                assert!(a != b);
            }
        }
    }
}
