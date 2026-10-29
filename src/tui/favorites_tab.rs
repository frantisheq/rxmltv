//! The "Favorites" tab: the shows marked with `*` in the Guide, and when they are on next.

use chrono::Utc;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::input::Input;
use super::{border_style, fit, parse_color, selected_style, Action};
use crate::config::UiOptions;
use crate::guide::Guide;
use crate::store::{title_key, Favorites};

pub struct FavoritesTab {
    state: ListState,
    /// Typing the title of a favorite to add.
    adding: Option<Input>,
}

impl FavoritesTab {
    pub fn new() -> Self {
        FavoritesTab { state: ListState::default(), adding: None }
    }

    pub fn captures_text(&self) -> bool {
        self.adding.is_some()
    }

    pub fn hints(&self) -> &'static str {
        if self.adding.is_some() {
            "type the title · Enter add · Esc cancel"
        } else {
            "↑↓ move · Enter show the next airing · a add a title · d remove · ? help · q quit"
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, favs: &Favorites) -> Action {
        if let Some(input) = self.adding.as_mut() {
            match key.code {
                KeyCode::Esc => self.adding = None,
                KeyCode::Enter => {
                    let title = input.value();
                    self.adding = None;
                    if !title.trim().is_empty() {
                        return Action::AddFavorite(title);
                    }
                }
                _ => {
                    let _ = input.handle(&key);
                }
            }
            return Action::None;
        }

        let len = favs.titles().len();
        let current = self.state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.select(current.saturating_sub(1), len),
            KeyCode::Down | KeyCode::Char('j') => self.select(current + 1, len),
            KeyCode::PageUp => self.select(current.saturating_sub(10), len),
            KeyCode::PageDown => self.select(current + 10, len),
            KeyCode::Home | KeyCode::Char('g') => self.select(0, len),
            KeyCode::End | KeyCode::Char('G') => self.select(len, len),
            KeyCode::Char('a') => self.adding = Some(Input::new("")),
            KeyCode::Char('d') | KeyCode::Delete => {
                if current < len {
                    return Action::RemoveFavorite(current);
                }
            }
            KeyCode::Enter => {
                if let Some(title) = favs.titles().get(current) {
                    return Action::ShowFavorite(title.clone());
                }
            }
            _ => {}
        }
        Action::None
    }

    fn select(&mut self, index: usize, len: usize) {
        self.state.select(if len == 0 { None } else { Some(index.min(len - 1)) });
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, favs: &Favorites, guide: Option<&Guide>, ui: &UiOptions) {
        // keep the selection valid after titles were added or removed
        let len = favs.titles().len();
        let selected = self.state.selected().unwrap_or(0).min(len.saturating_sub(1));
        self.state.select(if len == 0 { None } else { Some(selected) });

        let [list_area, detail_area] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Min(20)]).areas(area);

        let dot = Style::new().fg(parse_color(&ui.favorite_color));
        let width = list_area.width.saturating_sub(6) as usize;
        let items: Vec<ListItem<'static>> = favs
            .titles()
            .iter()
            .map(|t| ListItem::new(Line::from(vec![Span::styled("● ", dot), Span::raw(fit(t, width))])))
            .collect();

        let block = Block::bordered()
            .title(format!(" Favorites ({len}) "))
            .border_style(border_style(true));
        if items.is_empty() {
            f.render_widget(
                Paragraph::new("No favorites yet. Press * on a programme in the Guide, or a here to type a title.")
                    .block(block),
                list_area,
            );
        } else {
            let list = List::new(items).block(block).highlight_style(selected_style()).highlight_symbol("");
            f.render_stateful_widget(list, list_area, &mut self.state);
        }

        // the airings of the selected favorite
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let (Some(title), Some(g)) = (self.state.selected().and_then(|i| favs.titles().get(i)), guide) {
            lines.push(Line::styled(title.clone(), Style::new().add_modifier(Modifier::BOLD)));
            let airings = g.airings(&title_key(title));
            let now = Utc::now().timestamp();
            let upcoming: Vec<&(usize, usize)> = airings
                .iter()
                .filter(|&&(c, p)| g.channels[c].end(p).map_or(true, |end| end.timestamp() > now))
                .collect();
            if upcoming.is_empty() {
                lines.push(Line::styled("Not in the guide from now on.", Style::new().fg(Color::DarkGray)));
            } else {
                lines.push(Line::styled(
                    format!("{} upcoming in the guide:", upcoming.len()),
                    Style::new().fg(Color::Cyan),
                ));
                for &&(c, p) in upcoming.iter().take(30) {
                    let channel = &g.channels[c];
                    let programme = &channel.programmes[p];
                    lines.push(Line::raw(format!(
                        "{}  {} {}",
                        programme.start.format("%a %d.%m. %H:%M"),
                        channel.country,
                        channel.name
                    )));
                }
            }
        } else if guide.is_none() && len > 0 {
            lines.push(Line::styled("Load a guide to see when the favorites are on.", Style::new().fg(Color::DarkGray)));
        }
        f.render_widget(
            Paragraph::new(lines).block(Block::bordered().title(" Next airings ").border_style(border_style(false))),
            detail_area,
        );

        if let Some(input) = &self.adding {
            let popup = super::centered(area, area.width.saturating_sub(8).min(70), 4);
            f.render_widget(ratatui::widgets::Clear, popup);
            let block = Block::bordered().title(" Add a favorite: title ").border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(popup);
            f.render_widget(block, popup);
            f.render_widget(Paragraph::new(Line::from(input.spans(inner.width.saturating_sub(1) as usize))), inner);
        }
    }
}
