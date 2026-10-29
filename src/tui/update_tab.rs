//! The "Update" tab: runs the generator in the background and shows its progress and log.

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
use crate::{config, progress, stats, Options};

enum State {
    Idle,
    Running { started: Instant, cancel: Option<oneshot::Sender<()>> },
    Done { ok: bool, text: String, elapsed: Duration },
}

pub struct UpdateTab {
    state: State,
    rx: Option<Receiver<Result<u8, String>>>,
}

impl UpdateTab {
    pub fn new() -> Self {
        UpdateTab { state: State::Idle, rx: None }
    }

    pub fn hints() -> &'static str {
        "s start the update · x cancel it · ? help · q quit"
    }

    fn running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') | KeyCode::Enter => {
                if self.running() {
                    return Action::Status("An update is already running (x cancels it).".to_string());
                }
                return Action::StartUpdate;
            }
            KeyCode::Char('x') | KeyCode::Esc => self.cancel(),
            _ => {}
        }
        Action::None
    }

    /// Starts the generator on a helper thread. `block_on` there means the generator's future does not
    /// have to be `Send`, while the work it spawns still runs on the runtime's own threads.
    pub fn start(&mut self, opts: &Options, handle: &Handle) {
        if self.running() {
            return;
        }

        stats::reset();
        progress::reset();

        let opts = opts.clone();
        let handle = handle.clone();
        let (tx, rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        std::thread::spawn(move || {
            let result = handle.block_on(async move {
                tokio::select! {
                    r = crate::run_all(opts) => r.map_err(|e| e.to_string()),
                    _ = cancel_rx => Err("Cancelled.".to_string()),
                }
            });
            let _ = tx.send(result);
        });

        self.rx = Some(rx);
        self.state = State::Running { started: Instant::now(), cancel: Some(cancel_tx) };
    }

    pub fn cancel(&mut self) {
        if let State::Running { cancel, .. } = &mut self.state {
            if let Some(tx) = cancel.take() {
                let _ = tx.send(());
                progress::info("Cancelling ...");
            }
        }
    }

    /// Stops a running update and waits (briefly) until it has removed its temporary files.
    pub fn shutdown(&mut self) {
        self.cancel();
        if let Some(rx) = self.rx.take() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }

    /// Called regularly. Some(true) when an update has just written a guide that should be reloaded.
    pub fn poll(&mut self) -> Option<bool> {
        let received = match &self.rx {
            Some(rx) => match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Some(Err("The update stopped unexpectedly.".to_string())),
            },
            None => return None,
        };
        self.rx = None;

        let elapsed = match &self.state {
            State::Running { started, .. } => started.elapsed(),
            _ => Duration::ZERO,
        };
        let received_code = match &received {
            Some(Ok(code)) => Some(*code),
            _ => None,
        };
        let (ok, text) = match received {
            Some(Ok(0)) => (true, "Finished.".to_string()),
            Some(Ok(1)) => (false, "Nothing was written: see the log.".to_string()),
            Some(Ok(3)) => {
                (false, "The guide was written, but the command or the upload failed: see the log.".to_string())
            }
            Some(Ok(code)) => (false, format!("Stopped with exit code {code}.")),
            Some(Err(message)) => (false, message),
            None => (false, String::new()),
        };
        // the guide file is fine after a failed command or upload too, so it is reloaded then as well
        let reload = ok || matches!(received_code, Some(3));
        self.state = State::Done { ok, text, elapsed };
        Some(reload)
    }

    pub fn draw(&self, f: &mut Frame, area: Rect, o: &Options) {
        let [status_area, gauge_area, counts_area, log_area] = Layout::vertical([
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .areas(area);

        let snap = progress::snapshot();

        let (headline, style) = match &self.state {
            State::Idle => ("Ready. Press s to download the guide.".to_string(), Style::new()),
            State::Running { started, cancel } => {
                let what = if cancel.is_none() { "Cancelling" } else { "Updating" };
                (
                    format!("{what} ... {}", crate::format_duration(started.elapsed())),
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )
            }
            State::Done { ok, text, elapsed } => (
                format!("{text} ({})", crate::format_duration(*elapsed)),
                Style::new().fg(if *ok { Color::Green } else { Color::Red }).add_modifier(Modifier::BOLD),
            ),
        };

        let lines = vec![
            Line::styled(headline, style),
            Line::raw(format!("Writes {} for {} day(s) to {}", config::selection_summary(o), o.days, o.output)),
            Line::raw(format!("Cache: {}", o.cache_dir)),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" Update ")),
            status_area,
        );

        let (ratio, label) = if snap.total == 0 {
            (0.0, if self.running() { "Loading the channel list ...".to_string() } else { String::new() })
        } else {
            let label = if snap.current.is_empty() {
                format!("{}/{} channels", snap.done, snap.total)
            } else {
                format!("{}/{} channels, last: {}", snap.done, snap.total, snap.current)
            };
            ((snap.done as f64 / snap.total as f64).clamp(0.0, 1.0), label)
        };
        let block = Block::bordered().title(" Progress ");
        let inner = block.inner(gauge_area);
        f.render_widget(block, gauge_area);
        f.render_widget(Paragraph::new(progress_line(inner.width as usize, ratio, &label)), inner);

        let [cache, downloaded, no_data, failed] = stats::counts();
        f.render_widget(
            Paragraph::new(format!(
                " Files: {} from cache · {} downloaded · {} without data · {} failed",
                stats::n0(cache),
                stats::n0(downloaded),
                stats::n0(no_data),
                stats::n0(failed)
            ))
            .style(Style::new().fg(Color::DarkGray)),
            counts_area,
        );

        let height = log_area.height.saturating_sub(2) as usize;
        let log: Vec<Line<'static>> = progress::log_lines(height).into_iter().map(Line::raw).collect();
        f.render_widget(Paragraph::new(log).block(Block::bordered().title(" Log ")), log_area);
    }
}
