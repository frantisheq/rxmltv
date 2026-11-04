//! The "Share" tab: the HTTP server that shares the output file while the terminal UI is open.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use chrono::Local;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use super::Action;
use crate::serve::{self, Logger, RequestLog};
use crate::t;
use crate::Options;

const MAX_LOG_LINES: usize = 300;

enum State {
    Stopped,
    Starting,
    Running(Vec<String>),
    Failed(String),
}

enum Message {
    Started(Result<SocketAddr, String>),
    Request(RequestLog),
}

/// What a server was started with; when the settings differ from it, the server is restarted.
#[derive(PartialEq, Eq, Clone)]
struct Applied {
    bind: String,
    port: u16,
    file: PathBuf,
}

pub struct ShareTab {
    state: State,
    applied: Option<Applied>,
    rx: Option<Receiver<Message>>,
    cancel: Option<oneshot::Sender<()>>,
    log: VecDeque<String>,
    requests: usize,
    bytes: u64,
}

impl ShareTab {
    pub fn new() -> Self {
        ShareTab {
            state: State::Stopped,
            applied: None,
            rx: None,
            cancel: None,
            log: VecDeque::new(),
            requests: 0,
            bytes: 0,
        }
    }

    pub fn hints() -> &'static str {
        t!("s start / stop sharing · ? help · q quit")
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') | KeyCode::Enter => Action::ToggleShare,
            _ => Action::None,
        }
    }

    /// Called regularly: starts, restarts or stops the server so that it matches the settings.
    pub fn sync(&mut self, o: &Options, handle: &Handle) {
        let wanted = Applied {
            bind: o.serve_bind.trim().to_string(),
            port: o.serve_port,
            file: std::path::absolute(&o.output).unwrap_or_else(|_| PathBuf::from(&o.output)),
        };
        match (&self.applied, o.tui_serve) {
            (None, true) => self.start(wanted, handle),
            (Some(current), true) if *current != wanted => {
                self.stop();
                self.start(wanted, handle);
            }
            (Some(_), false) => self.stop(),
            _ => {}
        }
    }

    fn start(&mut self, wanted: Applied, handle: &Handle) {
        self.state = State::Starting;
        let (tx, rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let handle = handle.clone();
        let (bind, port, file) = (wanted.bind.clone(), wanted.port, wanted.file.clone());

        std::thread::spawn(move || {
            handle.block_on(async move {
                let listener = match serve::bind(&bind, port).await {
                    Ok(listener) => listener,
                    Err(e) => {
                        let _ = tx.send(Message::Started(Err(e)));
                        return;
                    }
                };
                match listener.local_addr() {
                    Ok(addr) => {
                        let _ = tx.send(Message::Started(Ok(addr)));
                    }
                    Err(e) => {
                        let _ = tx.send(Message::Started(Err(e.to_string())));
                        return;
                    }
                }

                let sender = Mutex::new(tx);
                let log: Logger = Arc::new(move |entry| {
                    if let Ok(sender) = sender.lock() {
                        let _ = sender.send(Message::Request(entry));
                    }
                });
                tokio::select! {
                    _ = serve::run(listener, file, log) => {}
                    _ = cancel_rx => {}
                }
            });
        });

        self.applied = Some(wanted);
        self.rx = Some(rx);
        self.cancel = Some(cancel_tx);
    }

    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.rx = None;
        self.applied = None;
        self.state = State::Stopped;
    }

    pub fn shutdown(&mut self) {
        self.stop();
    }

    /// Called regularly: picks up the server's start and its requests.
    pub fn poll(&mut self) {
        loop {
            let message = match &self.rx {
                Some(rx) => match rx.try_recv() {
                    Ok(message) => message,
                    Err(TryRecvError::Empty) => return,
                    Err(TryRecvError::Disconnected) => return,
                },
                None => return,
            };

            match message {
                Message::Started(Ok(addr)) => {
                    let name = self
                        .applied
                        .as_ref()
                        .and_then(|a| a.file.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_default();
                    self.state = State::Running(serve::urls(addr, &name));
                }
                Message::Started(Err(e)) => self.state = State::Failed(e),
                Message::Request(entry) => {
                    self.requests += 1;
                    self.bytes += entry.bytes;
                    if self.log.len() >= MAX_LOG_LINES {
                        self.log.pop_front();
                    }
                    self.log.push_back(format!(
                        "{}  {}  {}  {}  {}",
                        Local::now().format("%H:%M:%S"),
                        entry.peer,
                        entry.request,
                        entry.status,
                        crate::upload::human_size(entry.bytes)
                    ));
                }
            }
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect, o: &Options) {
        let [status_area, log_area] = Layout::vertical([Constraint::Length(8), Constraint::Min(3)]).areas(area);

        let (headline, style) = match &self.state {
            State::Stopped => (t!("Not sharing. Press s to start.").to_string(), Style::new()),
            State::Starting => (t!("Starting ...").to_string(), Style::new().fg(Color::Yellow)),
            State::Running(_) => (
                t!("Sharing over HTTP.").to_string(),
                Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            State::Failed(e) => (e.clone(), Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
        };

        let file = std::path::absolute(&o.output).unwrap_or_else(|_| PathBuf::from(&o.output));
        let size = std::fs::metadata(&file)
            .map(|m| crate::upload::human_size(m.len()))
            .unwrap_or_else(|_| t!("missing").to_string());

        let mut lines = vec![
            Line::styled(headline, style),
            Line::raw(t!("File: {path} ({size})", path = file.display(), size = size)),
        ];
        if let State::Running(urls) = &self.state {
            lines.push(Line::styled(t!("Addresses:"), Style::new().fg(Color::Cyan)));
            for url in urls {
                lines.push(Line::raw(format!("  {url}")));
            }
        }
        lines.push(Line::styled(
            t!(
                "Requests: {count} · sent {size}",
                count = self.requests,
                size = crate::upload::human_size(self.bytes)
            ),
            Style::new().fg(Color::DarkGray),
        ));
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(format!(" {} ", t!("Share")))),
            status_area,
        );

        let height = log_area.height.saturating_sub(2) as usize;
        let skip = self.log.len().saturating_sub(height);
        let rows: Vec<Line<'static>> = self.log.iter().skip(skip).cloned().map(Line::raw).collect();
        f.render_widget(
            Paragraph::new(rows).block(Block::bordered().title(format!(" {} ", t!("Requests")))),
            log_area,
        );
    }
}
