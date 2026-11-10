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
use crate::{config, t, Options};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Language,
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
    EnglishGenres,
    SlovakGenres,
    GuideLayout,
    ShowCountry,
    ShowQuality,
    ShowNew,
    ShowRerun,
    Primetime,
    HighlightPrimetime,
    FavoriteColor,
    GenreColors,
    NotifyColor,
    NotifyPopup,
    NotifySound,
    NotifyApprise,
    AppriseCommand,
    AppriseUrls,
    NotifyMinutes,
    NotifyRepeat,
    NotifyScope,
    NotifyTest,
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
    TuiServe,
    ServePort,
    ServeBind,
    UpdateTimes,
    ServiceServe,
    Save,
    Run,
    UploadNow,
    PurgeCache,
}

const ITEMS: [Item; 57] = [
    Item::Language,
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
    Item::EnglishGenres,
    Item::SlovakGenres,
    Item::GuideLayout,
    Item::ShowCountry,
    Item::ShowQuality,
    Item::ShowNew,
    Item::ShowRerun,
    Item::Primetime,
    Item::HighlightPrimetime,
    Item::FavoriteColor,
    Item::GenreColors,
    Item::NotifyColor,
    Item::NotifyPopup,
    Item::NotifySound,
    Item::NotifyApprise,
    Item::AppriseCommand,
    Item::AppriseUrls,
    Item::NotifyMinutes,
    Item::NotifyRepeat,
    Item::NotifyScope,
    Item::NotifyTest,
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
    Item::TuiServe,
    Item::ServePort,
    Item::ServeBind,
    Item::UpdateTimes,
    Item::ServiceServe,
    Item::Save,
    Item::Run,
    Item::UploadNow,
    Item::PurgeCache,
];

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Language => t!("Language"),
            Item::Days => t!("Days to download"),
            Item::Output => t!("Output file"),
            Item::CacheDir => t!("Cache folder"),
            Item::Concurrency => t!("Parallel downloads"),
            Item::Groups => t!("Channel groups"),
            Item::Channels => t!("Individual channels"),
            Item::CleanCache => t!("Clean the cache first"),
            Item::CompressCache => t!("Compress cache files"),
            Item::StripHdTv => t!("Strip HD / TV from names"),
            Item::KeepTitleNumbers => t!("Keep numbers in titles"),
            Item::KeepEmptyChannels => t!("Write empty channels"),
            Item::NoImages => t!("No programme posters"),
            Item::RetryMissing => t!("Retry missing items"),
            Item::RefreshChannels => t!("Refresh channel list"),
            Item::EnglishGenres => t!("English genre names"),
            Item::SlovakGenres => t!("Slovak genre names"),
            Item::GuideLayout => t!("Guide layout"),
            Item::ShowCountry => t!("Show country codes"),
            Item::ShowQuality => t!("Show quality tag (HDTV)"),
            Item::ShowNew => t!("Show NEW tag"),
            Item::ShowRerun => t!("Show RERUN tag"),
            Item::Primetime => t!("Primetime"),
            Item::HighlightPrimetime => t!("Highlight primetime"),
            Item::FavoriteColor => t!("Favorite dot colour"),
            Item::GenreColors => t!("Genre dot colours"),
            Item::NotifyColor => t!("Notification colour"),
            Item::NotifyPopup => t!("Notifications: popup"),
            Item::NotifySound => t!("Notifications: sound"),
            Item::NotifyApprise => t!("Notifications: Apprise"),
            Item::AppriseCommand => t!("Apprise command"),
            Item::AppriseUrls => t!("Apprise URLs"),
            Item::NotifyMinutes => t!("Notify minutes before"),
            Item::NotifyRepeat => t!("Notify: repeat"),
            Item::NotifyScope => t!("Notify: channels"),
            Item::NotifyTest => t!("Send a test notification"),
            Item::PostCommand => t!("Command after update"),
            Item::UploadAuto => t!("Upload after update"),
            Item::UploadProtocol => t!("Upload: protocol"),
            Item::UploadHost => t!("Upload: server"),
            Item::UploadPort => t!("Upload: port"),
            Item::UploadUser => t!("Upload: user name"),
            Item::UploadPassword => t!("Upload: password"),
            Item::UploadFolder => t!("Upload: folder"),
            Item::UploadName => t!("Upload: file name"),
            Item::UploadKey => t!("Upload: SSH key file"),
            Item::UploadPassphrase => t!("Upload: key passphrase"),
            Item::UploadInsecure => t!("Skip certificate check"),
            Item::TuiServe => t!("Share over HTTP"),
            Item::ServePort => t!("HTTP port"),
            Item::ServeBind => t!("HTTP address"),
            Item::UpdateTimes => t!("Update times (service)"),
            Item::ServiceServe => t!("Service: share over HTTP"),
            Item::Save => t!("Save settings"),
            Item::Run => t!("Run the update now"),
            Item::UploadNow => t!("Upload now"),
            Item::PurgeCache => t!("Purge the cache now"),
        }
    }

    fn help(self) -> &'static str {
        match self {
            Item::Language => t!("The language of the interface and of the genre names. Enter or Left/Right change it."),
            Item::Days => t!("How many days to download, starting today. Left/Right change it by one."),
            Item::Output => t!("Where the XMLTV file is written. A name ending in .gz is compressed. The Guide tab reads this file."),
            Item::CacheDir => t!("Downloaded files are kept here, so later runs only fetch what is new."),
            Item::Concurrency => t!("Downloads running at the same time. Be considerate to the server. Left/Right change it by one."),
            Item::Groups => t!("Enter opens the group picker. No groups and no channels picked means every channel."),
            Item::Channels => t!("Enter opens the channel picker: pick single channels, and optionally give them an XMLTV id of your own."),
            Item::CleanCache => t!("Before updating, delete cached files older than today and leftover temporary files."),
            Item::CompressCache => t!("Store cache files gzip-compressed to save disk space (usually about a fifth of the size). Existing files are compressed at the next update; both kinds are always readable."),
            Item::StripHdTv => t!("Remove the separate words HD and TV from channel names and ids (Nova HD becomes Nova). Channel ids you picked change with it."),
            Item::KeepTitleNumbers => t!("Leave (3), (2/6) and roman season numbers in the title instead of writing them as episode numbers."),
            Item::KeepEmptyChannels => t!("Also write channels that have no programmes."),
            Item::NoImages => t!("Do not write poster <icon> elements."),
            Item::RetryMissing => t!("Ask the server again for items it had no data for (normally skipped for 12 hours)."),
            Item::RefreshChannels => t!("Download the channel list again instead of using the cached channels.xml (it is otherwise never refreshed)."),
            Item::EnglishGenres => t!("Write an English name (<category lang=\"en\">) after each genre that has a known translation, in the XMLTV file. First letters are always capitalised."),
            Item::SlovakGenres => t!("Write a Slovak name (<category lang=\"sk\">) after each genre that has a known translation, in the XMLTV file."),
            Item::ShowCountry => t!("Show the country code (CZ, SK ...) before channel names in the Guide and Favorites tabs."),
            Item::GuideLayout => t!("list: channel list and one programme list. grid: one column per channel, like a newspaper listing, with the details below. Enter or Left/Right switch; v switches in the Guide."),
            Item::ShowQuality => t!("Show the quality tag (HDTV) after the show name in the Guide's programme list."),
            Item::ShowNew => t!("Show the NEW tag after the show name in the Guide's programme list."),
            Item::ShowRerun => t!("Show the RERUN tag after the show name in the Guide's programme list."),
            Item::Primetime => t!("The time primetime starts, as HH:MM (default 20:00). The programme that is on at that time is highlighted, and p jumps to it in the Guide."),
            Item::HighlightPrimetime => t!("Highlight the programme that is on at primetime in the Guide (list and grid)."),
            Item::FavoriteColor => t!("The colour of the dot before the time of a favorite show. A colour name (red, orange ...) or #rrggbb."),
            Item::GenreColors => t!("Enter opens the editor: which genre gets which dot colour (shown at the right edge of the programme list)."),
            Item::NotifyColor => t!("The colour of the dot before the time of a programme that has a notification. A colour name or #rrggbb."),
            Item::NotifyPopup => t!("Show a notification window in the terminal UI when a programme is about to start (Enter closes it)."),
            Item::NotifySound => t!("Sound a terminal bell with each notification in the terminal UI."),
            Item::NotifyApprise => t!("Also send each notification through Apprise (https://appriseit.com/). This works in the service mode too, with nobody logged in. Needs the apprise program."),
            Item::AppriseCommand => t!("The Apprise program, with options if you need them, e.g. apprise --tag family or apprise --config /etc/apprise.yml."),
            Item::AppriseUrls => t!("Apprise notification URLs, separated by spaces or commas (e.g. tgram://..., mailto://...). Empty: use Apprise's own configuration. Saved as plain text in the settings file."),
            Item::NotifyMinutes => t!("Default minutes before a programme starts that the notification comes (1 to 1440). Each notification can have its own. Left/Right change it by one."),
            Item::NotifyRepeat => t!("The default choice in the dialog when you press b: once (for the airing you chose) or every time (that airing and the upcoming ones). Enter or Left/Right change it."),
            Item::NotifyScope => t!("The default choice in the dialog: only the channel you pressed b on, or all channels (for every time notifications). Enter or Left/Right change it."),
            Item::NotifyTest => t!("Send a test notification the way the settings above say (popup, sound, Apprise)."),
            Item::PostCommand => t!("A shell command run after a successful update, before the upload. {file} is the output file (also in $RXMLTV_OUTPUT). Empty: none."),
            Item::UploadAuto => t!("Upload the output file to the server below after every update. The Upload tab can always upload by hand. Needs the curl program."),
            Item::UploadProtocol => t!("ftp, ftps (FTP with TLS), sftp (over SSH), webdav (http) or webdavs (https). Enter or Left/Right change it."),
            Item::UploadHost => t!("The server's name or address."),
            Item::UploadPort => t!("Empty: the protocol's usual port (21 FTP/FTPS, 22 SFTP, 80 WebDAV, 443 WebDAVs)."),
            Item::UploadUser => t!("The user name to log in with. Empty: none."),
            Item::UploadPassword => t!("Saved as plain text in the settings file (readable only by you on Linux/macOS). Empty: $RXMLTV_UPLOAD_PASSWORD, if set."),
            Item::UploadFolder => t!("The folder on the server. A leading / is an absolute path (FTP, SFTP); otherwise it is relative to the login folder. Folders are created (FTP, SFTP). WebDAV folders must exist."),
            Item::UploadName => t!("The file name on the server. Empty: the name of the output file."),
            Item::UploadKey => t!("SFTP: the SSH private key to log in with. Enter browses for the file, e types the path. Empty: log in with the password."),
            Item::UploadPassphrase => t!("SFTP: the passphrase of the SSH key, if it has one. Saved as plain text in the settings file."),
            Item::UploadInsecure => t!("Do not verify the TLS certificate (FTPS, https) or the SSH host key (SFTP). Only for servers you trust, e.g. a self-signed certificate."),
            Item::TuiServe => t!("Share the output file over HTTP while the terminal UI is open (see the Share tab). To share without the UI, run rxmltv --serve."),
            Item::ServePort => t!("The port of the HTTP server (1 to 65535). Ports below 1024 usually need extra rights. Changes apply at once."),
            Item::ServeBind => t!("The address the HTTP server listens on: 0.0.0.0 (all IPv4 networks), :: (all, including IPv6) or 127.0.0.1 (only this computer). The file is public to everyone who can reach it."),
            Item::UpdateTimes => t!("Service mode: the times of day (HH:MM, separated by commas) when the guide is updated, e.g. 04:30, 16:00. Empty: no scheduled update. A missed update is made up when the service starts."),
            Item::ServiceServe => t!("Service mode: also share the output file over HTTP (port and address as above)."),
            Item::Save => t!("Write these settings to the settings file. `rxmltv --config <file>` then uses them without the terminal UI."),
            Item::Run => t!("Start the update with the settings as they are now (saved or not), then the command and upload if set."),
            Item::UploadNow => t!("Upload the output file now, with a progress bar on the Upload tab."),
            Item::PurgeCache => t!("Delete every cached file, including the channel list (press Enter twice). The next update downloads everything again."),
        }
    }

    fn is_action(self) -> bool {
        matches!(self, Item::Save | Item::Run | Item::UploadNow | Item::PurgeCache | Item::NotifyTest)
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
                | Item::EnglishGenres
                | Item::SlovakGenres
                | Item::ShowCountry
                | Item::ShowQuality
                | Item::ShowNew
                | Item::ShowRerun
                | Item::HighlightPrimetime
                | Item::UploadAuto
                | Item::UploadInsecure
                | Item::TuiServe
                | Item::NotifyPopup
                | Item::NotifySound
                | Item::NotifyApprise
                | Item::ServiceServe
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
                | Item::Primetime
                | Item::NotifyColor
                | Item::AppriseCommand
                | Item::AppriseUrls
                | Item::NotifyMinutes
                | Item::UpdateTimes
                | Item::PostCommand
                | Item::UploadHost
                | Item::UploadPort
                | Item::UploadUser
                | Item::UploadPassword
                | Item::UploadFolder
                | Item::UploadName
                | Item::UploadKey
                | Item::UploadPassphrase
                | Item::ServePort
                | Item::ServeBind
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
        Item::EnglishGenres => Some(&mut o.english_genres),
        Item::SlovakGenres => Some(&mut o.slovak_genres),
        Item::ShowCountry => Some(&mut o.ui.show_country),
        Item::ShowQuality => Some(&mut o.ui.show_quality),
        Item::ShowNew => Some(&mut o.ui.show_new),
        Item::ShowRerun => Some(&mut o.ui.show_rerun),
        Item::HighlightPrimetime => Some(&mut o.ui.highlight_primetime),
        Item::UploadAuto => Some(&mut o.upload.enabled),
        Item::UploadInsecure => Some(&mut o.upload.insecure),
        Item::TuiServe => Some(&mut o.tui_serve),
        Item::NotifyPopup => Some(&mut o.notify_popup),
        Item::NotifySound => Some(&mut o.notify_sound),
        Item::NotifyApprise => Some(&mut o.notify_apprise),
        Item::ServiceServe => Some(&mut o.service_serve),
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
        Item::EnglishGenres => o.english_genres,
        Item::SlovakGenres => o.slovak_genres,
        Item::ShowCountry => o.ui.show_country,
        Item::ShowQuality => o.ui.show_quality,
        Item::ShowNew => o.ui.show_new,
        Item::ShowRerun => o.ui.show_rerun,
        Item::HighlightPrimetime => o.ui.highlight_primetime,
        Item::UploadAuto => o.upload.enabled,
        Item::UploadInsecure => o.upload.insecure,
        Item::TuiServe => o.tui_serve,
        Item::NotifyPopup => o.notify_popup,
        Item::NotifySound => o.notify_sound,
        Item::NotifyApprise => o.notify_apprise,
        Item::ServiceServe => o.service_serve,
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
        Item::Primetime => crate::config::format_clock(o.ui.primetime),
        Item::PostCommand => o.post_command.clone(),
        Item::UploadHost => o.upload.host.clone(),
        Item::UploadPort => o.upload.port.map(|p| p.to_string()).unwrap_or_default(),
        Item::UploadUser => o.upload.username.clone(),
        Item::UploadPassword => o.upload.password.clone(),
        Item::UploadFolder => o.upload.folder.clone(),
        Item::UploadName => o.upload.remote_name.clone(),
        Item::UploadKey => o.upload.key_file.clone(),
        Item::UploadPassphrase => o.upload.key_passphrase.clone(),
        Item::ServePort => o.serve_port.to_string(),
        Item::ServeBind => o.serve_bind.clone(),
        Item::NotifyColor => o.ui.notify_color.clone(),
        Item::AppriseCommand => o.apprise_command.clone(),
        Item::AppriseUrls => o.apprise_urls.clone(),
        Item::NotifyMinutes => o.notify_minutes.to_string(),
        Item::UpdateTimes => {
            o.update_times.iter().map(|m| crate::config::format_clock(*m)).collect::<Vec<_>>().join(", ")
        }
        _ => String::new(),
    }
}

/// What the menu shows after the label.
fn value_text(item: Item, o: &Options) -> String {
    let flag = |on: bool| (if on { "[x]" } else { "[ ]" }).to_string();
    let or_none = |text: String| if text.is_empty() { t!("(none)").to_string() } else { text };
    let or_unset = |text: String| if text.is_empty() { t!("(not set)").to_string() } else { text };
    match item {
        Item::Language => o.language.name().to_string(),
        Item::Groups => match &o.groups {
            Some(groups) => groups.join(", "),
            None => t!("(none picked)").to_string(),
        },
        Item::Channels => match o.channels.len() {
            0 => t!("(none picked)").to_string(),
            n => t!("{count} picked", count = n),
        },
        Item::GenreColors => t!("{count} rules", count = o.ui.genre_colors.len()),
        Item::GuideLayout => (if o.ui.grid_layout { t!("grid (channels as columns)") } else { t!("list") }).to_string(),
        Item::UploadProtocol => o.upload.protocol.label().to_string(),
        Item::UploadPort => match o.upload.port {
            Some(port) => port.to_string(),
            None => t!("{port} (usual)", port = o.upload.protocol.default_port()),
        },
        Item::UploadPassword | Item::UploadPassphrase => {
            if text_of(item, o).is_empty() {
                t!("(not set)").to_string()
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
                t!("(output file name)").to_string()
            } else {
                own
            }
        }
        Item::Days
        | Item::Output
        | Item::CacheDir
        | Item::Concurrency
        | Item::FavoriteColor
        | Item::Primetime
        | Item::ServePort
        | Item::ServeBind => text_of(item, o),
        Item::CleanCache
        | Item::CompressCache
        | Item::StripHdTv
        | Item::KeepTitleNumbers
        | Item::KeepEmptyChannels
        | Item::NoImages
        | Item::RetryMissing
        | Item::RefreshChannels
        | Item::EnglishGenres
        | Item::SlovakGenres
        | Item::ShowCountry
        | Item::ShowQuality
        | Item::ShowNew
        | Item::ShowRerun
        | Item::HighlightPrimetime
        | Item::UploadAuto
        | Item::UploadInsecure
        | Item::TuiServe
        | Item::NotifyPopup
        | Item::NotifySound
        | Item::NotifyApprise
        | Item::ServiceServe => flag(flag_get(item, o)),
        Item::NotifyRepeat => o.notify_repeat.label().to_string(),
        Item::NotifyScope => o.notify_scope.label().to_string(),
        Item::NotifyMinutes => t!("{minutes} min", minutes = o.notify_minutes),
        Item::NotifyColor => text_of(item, o),
        Item::AppriseCommand => text_of(item, o),
        Item::AppriseUrls => match crate::notify::split_urls(&o.apprise_urls).len() {
            0 => t!("(not set)").to_string(),
            n => t!("{count} URL(s)", count = n),
        },
        Item::UpdateTimes => or_none(text_of(item, o)),
        Item::Save | Item::Run | Item::UploadNow | Item::PurgeCache | Item::NotifyTest => String::new(),
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
            Err(t!("{what} must not be empty.", what = what))
        } else {
            Ok(text.to_string())
        }
    };
    match item {
        Item::Days => o.days = crate::parse_positive(t!("Days"), text)?,
        Item::Concurrency => o.concurrency = crate::parse_concurrency(t!("Parallel downloads"), text)?,
        Item::Output => {
            o.output = crate::util::expand_home(&not_empty(t!("The output file name"))?).to_string_lossy().to_string()
        }
        Item::CacheDir => o.cache_dir = not_empty(t!("The cache folder"))?,
        Item::FavoriteColor => {
            if !config::valid_color(text) {
                return Err(t!("Not a colour: use a name like red, orange, lightblue, or #rrggbb.").to_string());
            }
            o.ui.favorite_color = text.to_ascii_lowercase();
        }
        Item::Primetime => {
            o.ui.primetime = crate::config::parse_clock(text)
                .ok_or_else(|| t!("The time must be HH:MM, for example 20:00.").to_string())?;
        }
        Item::NotifyColor => {
            if !config::valid_color(text) {
                return Err(t!("Not a colour: use a name like red, orange, lightblue, or #rrggbb.").to_string());
            }
            o.ui.notify_color = text.to_ascii_lowercase();
        }
        Item::AppriseCommand => o.apprise_command = not_empty(t!("The Apprise command"))?,
        Item::AppriseUrls => o.apprise_urls = text.to_string(),
        Item::NotifyMinutes => {
            o.notify_minutes = text
                .parse::<u32>()
                .ok()
                .filter(|m| (1..=1440).contains(m))
                .ok_or_else(|| t!("The number of minutes must be from 1 to 1440.").to_string())?;
        }
        Item::UpdateTimes => {
            let mut times = Vec::new();
            for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                times.push(
                    config::parse_clock(part).ok_or_else(|| t!("Times must look like 04:30, 16:00.").to_string())?,
                );
            }
            o.update_times = times;
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
                        .ok_or_else(|| t!("The port must be a number from 1 to 65535.").to_string())?,
                )
            };
        }
        Item::ServePort => {
            o.serve_port = text
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| t!("The port must be a number from 1 to 65535.").to_string())?;
        }
        Item::ServeBind => o.serve_bind = not_empty(t!("The HTTP address"))?,
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
            t!("Enter accept · Esc cancel")
        } else {
            t!("↑↓ move · Enter edit / toggle / open · ←→ change a number · s save · ? help · q quit")
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
            Item::Concurrency => {
                o.concurrency = (o.concurrency as i64 + delta).clamp(1, crate::MAX_CONCURRENCY as i64) as usize
            }
            Item::UploadProtocol => o.upload.protocol = o.upload.protocol.step(delta as i32),
            Item::GuideLayout => o.ui.grid_layout = !o.ui.grid_layout,
            Item::NotifyMinutes => o.notify_minutes = (i64::from(o.notify_minutes) + delta).clamp(1, 1440) as u32,
            Item::NotifyRepeat => o.notify_repeat = o.notify_repeat.flip(),
            Item::NotifyScope => o.notify_scope = o.notify_scope.flip(),
            Item::Language => {
                o.language = o.language.step(delta as i32);
                crate::i18n::set(o.language); // the whole interface switches at once
            }
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
            Item::UploadProtocol | Item::GuideLayout | Item::Language | Item::NotifyRepeat | Item::NotifyScope => {
                self.adjust(o, 1);
                Action::None
            }
            Item::Save => Action::SaveConfig,
            Item::Run => Action::StartUpdate,
            Item::UploadNow => Action::StartUpload,
            Item::NotifyTest => Action::TestNotification,
            Item::PurgeCache => {
                if armed {
                    Action::PurgeCache
                } else {
                    self.purge_armed = true;
                    Action::Status(t!(
                        "This deletes ALL cached files in {path}. Press Enter again to do it, any other key to cancel.",
                        path = crate::util::expand_home(&o.cache_dir).display()
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
                    Action::Status(t!("Channel ids change with this option: check the picked channels again.").to_string())
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
                        Item::Save => t!("> {label} to {path}", label = item.label(), path = config_path.display()),
                        _ => format!("> {}", item.label()),
                    };
                    ListItem::new(Line::styled(text, Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)))
                } else {
                    let mut spans = vec![Span::raw(format!("{:<34}", item.label()))];
                    if item == Item::FavoriteColor {
                        spans.push(Span::styled("● ", Style::new().fg(parse_color(&o.ui.favorite_color))));
                    }
                    let value_style = if item.is_flag() { Style::new().fg(Color::Yellow) } else { Style::new() };
                    spans.push(Span::styled(value_text(item, o), value_style));
                    ListItem::new(Line::from(spans))
                }
            })
            .collect();

        let title = if self.dirty {
            format!(" {} ", t!("Settings (unsaved changes)"))
        } else {
            format!(" {} ", t!("Settings"))
        };
        let list = List::new(items)
            .block(Block::bordered().title(title).border_style(Style::new().fg(Color::Cyan)))
            .highlight_style(selected_style())
            .highlight_symbol("▸ ");
        f.render_stateful_widget(list, list_area, &mut self.state);

        let lines = vec![
            Line::styled(self.selected().help(), Style::new().fg(Color::White)),
            Line::raw(""),
            Line::from(vec![
                Span::styled(t!("Writes: "), Style::new().fg(Color::Cyan)),
                Span::raw(config::selection_summary(o)),
            ]),
            Line::from(vec![
                Span::styled(t!("Command: "), Style::new().fg(Color::Cyan)),
                Span::raw(config::command_line(o)),
            ]),
            Line::from(vec![
                Span::styled(t!("From the saved file: "), Style::new().fg(Color::Cyan)),
                Span::raw(format!("rxmltv --config {}", config_path.display())),
            ]),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(format!(" {} ", t!("About this setting")))),
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
            lines.push(Line::styled(t!("Enter accepts, Esc cancels"), Style::new().fg(Color::DarkGray)));
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
        // typing cannot go beyond what the arrow keys allow
        assert!(commit(Item::Concurrency, "64", &mut o).is_ok());
        assert!(commit(Item::Concurrency, "65", &mut o).is_err());
        assert_eq!(o.concurrency, 64);
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
