//! The genre colour editor, opened from the Settings tab: which genre gets which dot colour in the
//! programme list. The first rule that matches a programme's categories wins.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::input::Input;
use super::picker::Outcome;
use super::{fit, parse_color, selected_style};
use crate::config::{self, GenreColor};
use crate::groups::normalize;
use crate::Options;

enum Mode {
    Browse,
    /// Typing the keyword of a new rule.
    Keyword(Input),
    /// Typing the colour of a new rule with this keyword.
    NewColor(String, Input),
    /// Typing a new colour for the rule at this position.
    EditColor(usize, Input),
}

pub struct GenreEditor {
    state: ListState,
    mode: Mode,
    notice: Option<String>,
}

impl GenreEditor {
    pub fn open(o: &Options) -> Self {
        let mut state = ListState::default();
        if !o.ui.genre_colors.is_empty() {
            state.select(Some(0));
        }
        GenreEditor { state, mode: Mode::Browse, notice: None }
    }

    pub fn hints(&self) -> &'static str {
        match self.mode {
            Mode::Browse => "a add · Enter/e change colour · d delete · K/J move up/down · Esc back",
            _ => "Enter accept · Esc cancel",
        }
    }

    fn select(&mut self, index: i64, len: usize) {
        if len == 0 {
            self.state.select(None);
        } else {
            self.state.select(Some(index.clamp(0, len as i64 - 1) as usize));
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, o: &mut Options) -> Outcome {
        let mut out = Outcome { changed: false, close: false };

        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Browse => self.browse_key(key, o, &mut out),
            Mode::Keyword(mut input) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let keyword = normalize(input.value().trim());
                    if keyword.is_empty() {
                        self.notice = Some("A keyword is needed, for example sport.".to_string());
                        self.mode = Mode::Keyword(input);
                    } else {
                        self.mode = Mode::NewColor(keyword, Input::new(""));
                    }
                }
                _ => {
                    let _ = input.handle(&key);
                    self.mode = Mode::Keyword(input);
                }
            },
            Mode::NewColor(keyword, mut input) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let color = input.value().trim().to_ascii_lowercase();
                    if config::valid_color(&color) {
                        o.ui.genre_colors.push(GenreColor { keyword, color });
                        self.state.select(Some(o.ui.genre_colors.len() - 1));
                        out.changed = true;
                    } else {
                        self.notice = Some("Not a colour. Use a name from the list below, or #rrggbb.".to_string());
                        self.mode = Mode::NewColor(keyword, input);
                    }
                }
                _ => {
                    let _ = input.handle(&key);
                    self.mode = Mode::NewColor(keyword, input);
                }
            },
            Mode::EditColor(index, mut input) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let color = input.value().trim().to_ascii_lowercase();
                    if !config::valid_color(&color) {
                        self.notice = Some("Not a colour. Use a name from the list below, or #rrggbb.".to_string());
                        self.mode = Mode::EditColor(index, input);
                    } else if let Some(rule) = o.ui.genre_colors.get_mut(index) {
                        rule.color = color;
                        out.changed = true;
                    }
                }
                _ => {
                    let _ = input.handle(&key);
                    self.mode = Mode::EditColor(index, input);
                }
            },
        }
        out
    }

    fn browse_key(&mut self, key: KeyEvent, o: &mut Options, out: &mut Outcome) {
        self.notice = None;
        let len = o.ui.genre_colors.len();
        let current = self.state.selected().unwrap_or(0);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => out.close = true,
            KeyCode::Up if shift => self.move_rule(o, out, -1),
            KeyCode::Down if shift => self.move_rule(o, out, 1),
            KeyCode::Char('K') => self.move_rule(o, out, -1),
            KeyCode::Char('J') => self.move_rule(o, out, 1),
            KeyCode::Up | KeyCode::Char('k') => self.select(current as i64 - 1, len),
            KeyCode::Down | KeyCode::Char('j') => self.select(current as i64 + 1, len),
            KeyCode::Home | KeyCode::Char('g') => self.select(0, len),
            KeyCode::End | KeyCode::Char('G') => self.select(len as i64, len),
            KeyCode::Char('a') => self.mode = Mode::Keyword(Input::new("")),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(rule) = o.ui.genre_colors.get(current) {
                    self.mode = Mode::EditColor(current, Input::new(&rule.color));
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if current < len {
                    o.ui.genre_colors.remove(current);
                    out.changed = true;
                    let len = o.ui.genre_colors.len();
                    self.select(current as i64, len);
                }
            }
            _ => {}
        }
    }

    fn move_rule(&mut self, o: &mut Options, out: &mut Outcome, delta: i64) {
        let Some(from) = self.state.selected() else { return };
        let to = from as i64 + delta;
        if to < 0 || to >= o.ui.genre_colors.len() as i64 {
            return;
        }
        o.ui.genre_colors.swap(from, to as usize);
        self.state.select(Some(to as usize));
        out.changed = true;
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, o: &Options, categories: &[(String, usize)]) {
        let block = Block::bordered()
            .title(" Genre colours: the first matching rule gives the dot its colour ")
            .border_style(Style::new().fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let [main, bottom] = Layout::vertical([Constraint::Min(3), Constraint::Length(5)]).areas(inner);
        let [rules_area, seen_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

        // the rules
        let items: Vec<ListItem<'static>> = o
            .ui
            .genre_colors
            .iter()
            .map(|r| {
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::new().fg(parse_color(&r.color))),
                    Span::raw(format!("{:<18}", fit(&r.keyword, 18))),
                    Span::styled(r.color.clone(), Style::new().fg(Color::DarkGray)),
                ]))
            })
            .collect();
        let rules_block = Block::bordered().title(format!(" Rules ({}) ", items.len()));
        if items.is_empty() {
            f.render_widget(Paragraph::new("No rules: no genre dots. Press a to add one.").block(rules_block), rules_area);
        } else {
            let list = List::new(items).block(rules_block).highlight_style(selected_style()).highlight_symbol("");
            f.render_stateful_widget(list, rules_area, &mut self.state);
        }

        // the genres that occur in the loaded guide, with the colour they get now
        let seen: Vec<Line<'static>> = if categories.is_empty() {
            vec![Line::styled("Load a guide to see its genres here.", Style::new().fg(Color::DarkGray))]
        } else {
            categories
                .iter()
                .map(|(name, count)| {
                    let dot = match o.ui.genre_color(&normalize(name)) {
                        Some(color) => Span::styled("● ", Style::new().fg(parse_color(color))),
                        None => Span::raw("  "),
                    };
                    Line::from(vec![dot, Span::raw(format!("{name} ({count})"))])
                })
                .collect()
        };
        f.render_widget(
            Paragraph::new(seen).block(Block::bordered().title(" Genres in the guide ")),
            seen_area,
        );

        // an input line, a notice, and the colour names
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.mode {
            Mode::Browse => {
                if let Some(notice) = &self.notice {
                    lines.push(Line::styled(notice.clone(), Style::new().fg(Color::Yellow)));
                }
            }
            Mode::Keyword(input) => {
                let mut spans = vec![Span::styled("Keyword (part of a genre, e.g. sport): ", Style::new().fg(Color::Yellow))];
                spans.extend(input.spans(bottom.width.saturating_sub(42) as usize));
                lines.push(Line::from(spans));
            }
            Mode::NewColor(keyword, input) => {
                let mut spans = vec![Span::styled(format!("Colour for \"{keyword}\": "), Style::new().fg(Color::Yellow))];
                spans.extend(input.spans(bottom.width.saturating_sub(keyword.chars().count() as u16 + 16) as usize));
                lines.push(Line::from(spans));
            }
            Mode::EditColor(_, input) => {
                let mut spans = vec![Span::styled("New colour: ", Style::new().fg(Color::Yellow))];
                spans.extend(input.spans(bottom.width.saturating_sub(14) as usize));
                lines.push(Line::from(spans));
            }
        }
        if !matches!(self.mode, Mode::Browse) {
            if let Some(notice) = &self.notice {
                lines.push(Line::styled(notice.clone(), Style::new().fg(Color::Red)));
            }
        }

        let mut names: Vec<Span<'static>> = vec![Span::styled("Colours: ", Style::new().add_modifier(Modifier::BOLD))];
        for name in config::COLOR_NAMES {
            names.push(Span::styled("● ", Style::new().fg(parse_color(name))));
            names.push(Span::raw(format!("{name}  ")));
        }
        names.push(Span::raw("or #rrggbb"));
        lines.push(Line::from(names));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), bottom);
    }
}
