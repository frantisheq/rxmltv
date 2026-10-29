//! The "Upload" tab: sends the generated guide to the server set in Settings, with a progress bar and a log.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use super::{progress_line, Action};
use crate::upload::{self, Event};
use crate::Options;

const MAX_LOG_LINES: usize = 500;

enum State {
    Idle,
    Running { started: Instant, cancel: Option<oneshot::Sender<()>> },
    Done { ok: bool, text: String, elapsed: Duration },
}

enum Message {
    Event(Event),
    Finished(Result<(), String>),
}

pub struct UploadTab {
    state: State,
    rx: Option<Receiver<Message>>,
    log: Vec<String>,
    percent: f64,
}

impl UploadTab {
    pub fn new() -> Self {
        UploadTab { state: State::Idle, rx: None, log: Vec::new(), percent: 0.0 }
    }

    pub fn hints() -> &'static str {
        "s upload now · x cancel · ? help · q quit"
    }

    fn running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') | KeyCode::Enter => {
                if self.running() {
                    return Action::Status("An upload is already running (x cancels it).".to_string());
                }
                return Action::StartUpload;
            }
            KeyCode::Char('x') | KeyCode::Esc => self.cancel(),
            _ => {}
        }
        Action::None
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= MAX_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line);
    }

    pub fn start(&mut self, o: &Options, handle: &Handle) {
        if self.running() {
            return;
        }
        self.log.clear();
        self.percent = 0.0;

        let file = match std::path::absolute(&o.output) {
            Ok(file) => file,
            Err(e) => {
                self.state = State::Done { ok: false, text: format!("Bad output path: {e}"), elapsed: Duration::ZERO };
                return;
            }
        };

        let settings = o.upload.clone();
        let handle = handle.clone();
        let (tx, rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        std::thread::spawn(move || {
            let events = tx.clone();
            let result = handle.block_on(async move {
                tokio::select! {
                    r = upload::upload(&settings, &file, move |event| {
                        let _ = events.send(Message::Event(event));
                    }) => r,
                    _ = cancel_rx => Err("Cancelled.".to_string()),
                }
            });
            let _ = tx.send(Message::Finished(result));
        });

        self.rx = Some(rx);
        self.state = State::Running { started: Instant::now(), cancel: Some(cancel_tx) };
    }

    pub fn cancel(&mut self) {
        if let State::Running { cancel, .. } = &mut self.state {
            if let Some(tx) = cancel.take() {
                let _ = tx.send(());
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.cancel();
        if let Some(rx) = self.rx.take() {
            let _ = rx.recv_timeout(Duration::from_secs(3));
        }
    }

    /// Called regularly: picks up progress and the end of the upload.
    pub fn poll(&mut self) {
        loop {
            let message = match &self.rx {
                Some(rx) => match rx.try_recv() {
                    Ok(message) => message,
                    Err(TryRecvError::Empty) => return,
                    Err(TryRecvError::Disconnected) => {
                        self.rx = None;
                        self.state = State::Done {
                            ok: false,
                            text: "The upload stopped unexpectedly.".to_string(),
                            elapsed: Duration::ZERO,
                        };
                        return;
                    }
                },
                None => return,
            };

            match message {
                Message::Event(Event::Log(line)) => self.push_log(line),
                Message::Event(Event::Progress(percent)) => self.percent = percent,
                Message::Finished(result) => {
                    self.rx = None;
                    let elapsed = match &self.state {
                        State::Running { started, .. } => started.elapsed(),
                        _ => Duration::ZERO,
                    };
                    self.state = match result {
                        Ok(()) => State::Done { ok: true, text: "Uploaded.".to_string(), elapsed },
                        Err(text) => {
                            self.push_log(format!("Error: {text}"));
                            State::Done { ok: false, text, elapsed }
                        }
                    };
                    return;
                }
            }
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect, o: &Options) {
        let [status_area, bar_area, log_area] =
            Layout::vertical([Constraint::Length(6), Constraint::Length(3), Constraint::Min(3)]).areas(area);

        let (headline, style) = match &self.state {
            State::Idle => ("Ready. Press s to upload the guide.".to_string(), Style::new()),
            State::Running { started, cancel } => (
                format!(
                    "{} ... {}",
                    if cancel.is_none() { "Cancelling" } else { "Uploading" },
                    crate::format_duration(started.elapsed())
                ),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            State::Done { ok, text, elapsed } => (
                format!("{text} ({})", crate::format_duration(*elapsed)),
                Style::new().fg(if *ok { Color::Green } else { Color::Red }).add_modifier(Modifier::BOLD),
            ),
        };

        let file = std::path::absolute(&o.output).unwrap_or_else(|_| o.output.clone().into());
        let target = match o.upload.remote_url(&o.upload.target_name(&file)) {
            Ok(url) => url,
            Err(e) => format!("({e} Set the server on the Settings tab.)"),
        };
        let size = std::fs::metadata(&file).map(|m| upload::human_size(m.len())).unwrap_or_else(|_| "missing".to_string());

        let lines = vec![
            Line::styled(headline, style),
            Line::raw(format!("File: {} ({size})", file.display())),
            Line::raw(format!("To:   {target}")),
            Line::raw(format!(
                "As:   {}  ·  {}",
                if o.upload.username.trim().is_empty() { "(no user name)" } else { o.upload.username.trim() },
                o.upload.protocol.label()
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" Upload ")),
            status_area,
        );

        let label = match (&self.state, self.percent) {
            (State::Idle, _) => String::new(),
            (_, p) => format!("{p:.0}%"),
        };
        let block = Block::bordered().title(" Progress ");
        let inner = block.inner(bar_area);
        f.render_widget(block, bar_area);
        f.render_widget(
            Paragraph::new(progress_line(inner.width as usize, self.percent / 100.0, &label)),
            inner,
        );

        let height = log_area.height.saturating_sub(2) as usize;
        let skip = self.log.len().saturating_sub(height);
        let log: Vec<Line<'static>> = self.log.iter().skip(skip).cloned().map(Line::raw).collect();
        f.render_widget(Paragraph::new(log).block(Block::bordered().title(" Log ")), log_area);
    }
}
