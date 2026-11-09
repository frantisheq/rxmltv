//! The "Notifications" tab: the list of programmes that notify, and how each one does it.

use chrono::Utc;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::input::Input;
use super::{border_style, centered, fit, parse_color, selected_style, Action};
use crate::guide::Guide;
use crate::notify::{Fired, Repeat, Rule, Rules, Scope};
use crate::t;
use crate::Options;

/// The rules and what has been announced; shared with the Guide tab (the dots) and the notification check.
pub struct NotifyState {
    pub rules: Rules,
    pub fired: Fired,
}

impl NotifyState {
    pub fn new() -> Self {
        let dir = crate::config::app_dir();
        NotifyState {
            rules: Rules::load(dir.join("rxmltv.notifications")),
            fired: Fired::load(dir.join("rxmltv.notified")),
        }
    }
}

enum Mode {
    Browse,
    /// Typing the title of a notification to add.
    Add(Input),
    /// Typing the minutes before the start for the rule with this id.
    Minutes(u64, Input),
}

pub struct NotificationsTab {
    state: ListState,
    mode: Mode,
    notice: Option<String>,
}

impl NotificationsTab {
    pub fn new() -> Self {
        NotificationsTab { state: ListState::default(), mode: Mode::Browse, notice: None }
    }

    pub fn captures_text(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    pub fn hints(&self) -> &'static str {
        match self.mode {
            Mode::Browse => t!("↑↓ move · Enter show next airing · a add · d delete · r once/every time · c this channel/all · m minutes · R re-arm · t test · ? help"),
            Mode::Add(_) => t!("type the title · Enter add · Esc cancel"),
            Mode::Minutes(..) => t!("type minutes before the start (empty = default) · Enter accept · Esc cancel"),
        }
    }

    fn selected_rule<'a>(&self, state: &'a NotifyState) -> Option<&'a Rule> {
        self.state.selected().and_then(|i| state.rules.rules().get(i))
    }

    pub fn on_key(&mut self, key: KeyEvent, state: &mut NotifyState, guide: Option<&Guide>) -> Action {
        self.notice = None;
        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Add(mut input) => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        let title = input.value();
                        if !title.trim().is_empty() {
                            // typed by hand there is no airing: every time from now on, on all channels
                            let now = Utc::now().timestamp();
                            if let Err(e) =
                                state.rules.add(&title, Repeat::Always, Scope::AllChannels, "", "", None, now)
                            {
                                return Action::Status(e);
                            }
                        }
                    }
                    _ => {
                        let _ = input.handle(&key);
                        self.mode = Mode::Add(input);
                    }
                }
                Action::None
            }
            Mode::Minutes(id, mut input) => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        let text = input.value();
                        let text = text.trim();
                        let minutes = if text.is_empty() {
                            Ok(None)
                        } else {
                            match text.parse::<u32>() {
                                Ok(m) if (1..=1440).contains(&m) => Ok(Some(m)),
                                _ => Err(()),
                            }
                        };
                        match minutes {
                            Ok(minutes) => {
                                if let Err(e) = state.rules.update(id, |r| r.minutes = minutes) {
                                    return Action::Status(e);
                                }
                            }
                            Err(()) => {
                                self.notice = Some(t!("The number of minutes must be from 1 to 1440.").to_string());
                                self.mode = Mode::Minutes(id, input);
                            }
                        }
                    }
                    _ => {
                        let _ = input.handle(&key);
                        self.mode = Mode::Minutes(id, input);
                    }
                }
                Action::None
            }
            Mode::Browse => self.browse_key(key, state, guide),
        }
    }

    fn browse_key(&mut self, key: KeyEvent, state: &mut NotifyState, guide: Option<&Guide>) -> Action {
        let len = state.rules.rules().len();
        let current = self.state.selected().unwrap_or(0);
        let selected = self.selected_rule(state).map(|r| (r.id, r.title.clone(), r.repeat, r.scope));

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.select(current.saturating_sub(1), len),
            KeyCode::Down | KeyCode::Char('j') => self.select(current + 1, len),
            KeyCode::PageUp => self.select(current.saturating_sub(10), len),
            KeyCode::PageDown => self.select(current + 10, len),
            KeyCode::Home | KeyCode::Char('g') => self.select(0, len),
            KeyCode::End | KeyCode::Char('G') => self.select(len, len),
            KeyCode::Char('a') => self.mode = Mode::Add(Input::new("")),
            KeyCode::Enter => {
                if let Some((_, title, _, _)) = selected {
                    return Action::ShowFavorite(title);
                }
            }
            KeyCode::Char('t') => return Action::TestNotification,
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some((id, ..)) = selected {
                    if let Err(e) = state.rules.remove(id) {
                        return Action::Status(e);
                    }
                    let _ = state.fired.clear_rule(id);
                }
            }
            KeyCode::Char('r') => {
                if let Some((id, _, repeat, _)) = selected {
                    let result = match repeat {
                        // for the airing it was set on, and the ones after it
                        Repeat::Once => state.rules.update(id, |r| r.repeat = Repeat::Always),
                        // a single notification needs an airing: the next one the rule covers
                        Repeat::Always => {
                            let now = Utc::now().timestamp();
                            let next = guide.and_then(|g| {
                                let rule = state.rules.rules().iter().find(|r| r.id == id)?;
                                next_airing(rule, g, now)
                            });
                            match next {
                                Some((channel_id, channel_name, start)) => state.rules.update(id, |r| {
                                    r.repeat = Repeat::Once;
                                    r.channel_id = channel_id;
                                    r.channel_name = channel_name;
                                    r.from = start;
                                }),
                                None => {
                                    return Action::Status(
                                        t!("No upcoming airing in the guide to make this a single notification.")
                                            .to_string(),
                                    )
                                }
                            }
                        }
                    };
                    if let Err(e) = result {
                        return Action::Status(e);
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some((id, _, repeat, scope)) = selected {
                    if repeat == Repeat::Once {
                        return Action::Status(
                            t!("A single notification is for one airing; it has no channel choice.").to_string(),
                        );
                    }
                    if let Err(e) = state.rules.update(id, |r| r.scope = scope.flip()) {
                        return Action::Status(e);
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Some(rule) = self.selected_rule(state) {
                    let current = rule.minutes.map(|m| m.to_string()).unwrap_or_default();
                    self.mode = Mode::Minutes(rule.id, Input::new(&current));
                }
            }
            KeyCode::Char('R') => {
                if let Some((id, ..)) = selected {
                    if let Err(e) = state.fired.clear_rule(id) {
                        return Action::Status(e);
                    }
                }
            }
            _ => {}
        }
        Action::None
    }

    fn select(&mut self, index: usize, len: usize) {
        self.state.select(if len == 0 { None } else { Some(index.min(len - 1)) });
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, state: &NotifyState, guide: Option<&Guide>, o: &Options) {
        let len = state.rules.rules().len();
        let selected = self.state.selected().unwrap_or(0).min(len.saturating_sub(1));
        self.state.select(if len == 0 { None } else { Some(selected) });

        let [list_area, info_area] = Layout::vertical([Constraint::Min(4), Constraint::Length(4)]).areas(area);

        let dot = Style::new().fg(parse_color(&o.ui.notify_color));
        let now = Utc::now().timestamp();
        let width = list_area.width.saturating_sub(6) as usize;
        let items: Vec<ListItem<'static>> = state
            .rules
            .rules()
            .iter()
            .map(|rule| {
                // a single notification is for one airing on one channel; "every time" can cover them all
                let scope = match (rule.repeat, rule.scope) {
                    (Repeat::Once, _) | (_, Scope::ThisChannel) if !rule.channel_name.is_empty() => {
                        rule.channel_name.clone()
                    }
                    (_, other) => other.label().to_string(),
                };
                let mode = match rule.repeat {
                    Repeat::Once => t!("once, {when}", when = when_of(rule.from)),
                    Repeat::Always => t!("every time from {when}", when = when_of(rule.from)),
                };
                let minutes = match rule.minutes {
                    Some(m) => t!("{minutes} min", minutes = m),
                    None => t!("default {minutes} min", minutes = o.notify_minutes),
                };
                let text = format!(
                    "{}  [{}]  [{}]  {}  {}",
                    rule.title,
                    scope,
                    mode,
                    minutes,
                    status(rule, &state.fired, guide, now)
                );
                ListItem::new(Line::from(vec![Span::styled("● ", dot), Span::raw(fit(&text, width))]))
            })
            .collect();

        let block = Block::bordered()
            .title(format!(" {} ", t!("Notifications ({count})", count = len)))
            .border_style(border_style(true));
        if items.is_empty() {
            f.render_widget(
                Paragraph::new(t!("No notifications yet. Press b on a programme in the Guide, or a here to type a title."))
                    .wrap(Wrap { trim: false })
                    .block(block),
                list_area,
            );
        } else {
            let list = List::new(items).block(block).highlight_style(selected_style()).highlight_symbol("");
            f.render_stateful_widget(list, list_area, &mut self.state);
        }

        let onoff = |on: bool| if on { t!("on") } else { t!("off") };
        let mut lines = vec![Line::styled(
            t!(
                "Delivery: popup {popup}, sound {sound}, Apprise {apprise}. Change it in Settings.",
                popup = onoff(o.notify_popup),
                sound = onoff(o.notify_sound),
                apprise = onoff(o.notify_apprise)
            ),
            Style::new().fg(Color::DarkGray),
        )];
        if let Some(notice) = &self.notice {
            lines.push(Line::styled(notice.clone(), Style::new().fg(Color::Red)));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered()), info_area);

        let prompt = match &self.mode {
            Mode::Add(input) => Some((t!("Add a notification: title"), input)),
            Mode::Minutes(_, input) => Some((t!("Minutes before the start (empty = default)"), input)),
            Mode::Browse => None,
        };
        if let Some((title, input)) = prompt {
            let popup = centered(area, area.width.saturating_sub(8).min(70), 4);
            f.render_widget(Clear, popup);
            let block = Block::bordered().title(format!(" {title} ")).border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(popup);
            f.render_widget(block, popup);
            f.render_widget(Paragraph::new(Line::from(input.spans(inner.width.saturating_sub(1) as usize))), inner);
        }
    }
}

/// "Thu 29.10. 20:15" for a timestamp (Prague time, like the guide).
pub fn when_of(ts: i64) -> String {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(d) => crate::i18n::when_short(&d.with_timezone(&chrono_tz::Europe::Prague)),
        None => String::new(),
    }
}

/// The first airing the rule is for that has not started yet: (channel id, channel name, start).
fn next_airing(rule: &Rule, guide: &Guide, now: i64) -> Option<(String, String, i64)> {
    guide.airings(&rule.key).iter().find_map(|&(c, p)| {
        let channel = &guide.channels[c];
        let start = channel.programmes[p].start.timestamp();
        (rule.covers(&channel.id, start) && start >= now - 60).then(|| (channel.id.clone(), channel.name.clone(), start))
    })
}

/// "waiting, next: Thu 29.10. 20:15 ČT1", "done" and so on.
fn status(rule: &Rule, fired: &Fired, guide: Option<&Guide>, now: i64) -> String {
    let announced = fired.for_rule(rule.id);

    match rule.repeat {
        Repeat::Once => {
            if !announced.is_empty() {
                return if rule.from < now { t!("done").to_string() } else { t!("announced").to_string() };
            }
            if rule.from < now - 60 {
                return t!("missed").to_string();
            }
            t!("waiting, next: {when}", when = format!("{} {}", when_of(rule.from), rule.channel_name))
        }
        Repeat::Always => {
            let Some(guide) = guide else { return String::new() };
            match next_airing(rule, guide, now) {
                Some((_, channel_name, start)) => {
                    t!("waiting, next: {when}", when = format!("{} {}", when_of(start), channel_name))
                }
                None => t!("not in the guide").to_string(),
            }
        }
    }
}
