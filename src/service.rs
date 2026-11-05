//! Service mode (`rxmltv --service`): no terminal UI, runs until stopped. It updates the guide at the times set in
//! `update_times`, can share the output file over HTTP, and sends the Apprise notifications for the
//! programmes in the notification list (see notify.rs), so notifications work with nobody logged in.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::{config, guide, notify, progress, t, BoxError, Options};

const CHECK_EVERY: Duration = Duration::from_secs(30);
const NOTIFY_EVERY: Duration = Duration::from_secs(20);

/// The most recent scheduled moment at or before `now` (looking back to yesterday).
pub fn last_scheduled<Tz: TimeZone>(times: &[u32], now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let today = now.date_naive();
    let mut best: Option<DateTime<Tz>> = None;
    for &minutes in times {
        for back in 0..=1u64 {
            let Some(day) = today.checked_sub_days(chrono::Days::new(back)) else { continue };
            let Some(naive) = day.and_hms_opt(minutes / 60, minutes % 60, 0) else { continue };
            let Some(at) = now.timezone().from_local_datetime(&naive).earliest() else { continue };
            if at <= now && best.as_ref().map_or(true, |b| at > *b) {
                best = Some(at);
            }
        }
    }
    best
}

/// Is a scheduled update due? `last_done` is when the guide was last written or an update last tried (a timestamp).
/// An update that was missed (the computer was off at 04:00) is made up as soon as the service runs.
pub fn update_due<Tz: TimeZone>(times: &[u32], now: DateTime<Tz>, last_done: Option<i64>) -> bool {
    match last_scheduled(times, now) {
        Some(scheduled) => last_done.map_or(true, |done| done < scheduled.timestamp()),
        None => false,
    }
}

fn modified_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
}

pub async fn run(opts: Options, _config_path: PathBuf) -> Result<u8, BoxError> {
    progress::set_plain(true);

    let times: Vec<String> = opts.update_times.iter().map(|m| config::format_clock(*m)).collect();
    progress::info(t!("Service started. Updates at {times}.", times = times.join(", ")));
    if times.is_empty() {
        progress::info(t!("No scheduled update times are set (update_times)."));
    }
    if opts.notify_apprise {
        progress::info(t!("Apprise notifications are on."));
    }

    let share = async {
        if opts.service_serve || opts.serve {
            if let Err(e) = crate::serve_forever(&opts).await {
                progress::warn(t!("Error: {error}", error = e));
            }
        }
        std::future::pending::<()>().await
    };

    tokio::join!(update_loop(&opts), notify_loop(&opts), share);
    Ok(0)
}

async fn update_loop(opts: &Options) {
    let mut last_attempt: Option<i64> = None;
    loop {
        let now = Local::now();
        let output = std::path::absolute(&opts.output).unwrap_or_else(|_| PathBuf::from(&opts.output));
        let last_done = modified_secs(&output).max(last_attempt);

        if update_due(&opts.update_times, now, last_done) {
            last_attempt = Some(now.timestamp());
            progress::info(t!("Scheduled update: starting."));

            let mut update = opts.clone();
            update.serve = false;
            update.service = false;
            match crate::run_all(update).await {
                Ok(0) => progress::info(t!("Scheduled update finished.")),
                Ok(code) => progress::warn(t!("Scheduled update stopped with exit code {code}.", code = code)),
                Err(e) => progress::warn(t!("Scheduled update failed: {error}", error = e)),
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

/// Sends the Apprise notifications that are due. Popups and sounds belong to the terminal UI; without Apprise
/// there is nothing for the service to do here.
async fn notify_loop(opts: &Options) {
    let dir = config::app_dir();
    let mut rules = notify::Rules::load(dir.join("rxmltv.notifications"));
    let mut fired = notify::Fired::load(dir.join("rxmltv.notified"));
    let mut guide: Option<guide::Guide> = None;
    let mut guide_time: Option<SystemTime> = None;

    loop {
        tokio::time::sleep(NOTIFY_EVERY).await;
        if !opts.notify_apprise {
            continue;
        }
        rules.reload();
        if rules.is_empty() {
            continue;
        }

        // read the guide again when an update has replaced the file
        let output = std::path::absolute(&opts.output).unwrap_or_else(|_| PathBuf::from(&opts.output));
        let modified = std::fs::metadata(&output).and_then(|m| m.modified()).ok();
        if guide.is_none() || modified != guide_time {
            guide_time = modified;
            let path = output.clone();
            guide = tokio::task::spawn_blocking(move || guide::load(&path).ok()).await.ok().flatten();
        }
        let Some(guide) = &guide else { continue };

        fired.reload();
        let now = Utc::now().timestamp();
        for due in notify::due(&rules, &fired, guide, now, opts.notify_minutes) {
            if due.claimed {
                continue; // announced already, by this service or by the terminal UI
            }
            match fired.claim(&due) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    progress::warn(e);
                    continue;
                }
            }

            let (title, body) = due.message();
            match notify::apprise(&opts.apprise_command, &opts.apprise_urls, &title, &body).await {
                Ok(()) => progress::info(t!("Notified: {title}", title = title)),
                Err(e) => {
                    fired.unclaim(&due.key()); // try again at the next check, while the programme is still ahead
                    progress::warn(t!("Notification failed ({title}): {error}", title = title, error = e));
                }
            }
        }
        fired.prune(now - 2 * 24 * 3600);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(day: u32, hour: u32, minute: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 10, day, hour, minute, 0).unwrap()
    }

    #[test]
    fn the_latest_scheduled_moment_is_found() {
        let times = [4 * 60, 16 * 60];
        assert_eq!(last_scheduled(&times, at(29, 10, 0)), Some(at(29, 4, 0)));
        assert_eq!(last_scheduled(&times, at(29, 16, 0)), Some(at(29, 16, 0)));
        assert_eq!(last_scheduled(&times, at(29, 17, 30)), Some(at(29, 16, 0)));
        // before the first time of the day: yesterday's last one
        assert_eq!(last_scheduled(&times, at(29, 3, 0)), Some(at(28, 16, 0)));
        assert_eq!(last_scheduled(&[], at(29, 3, 0)), None);
    }

    #[test]
    fn an_update_is_due_when_the_guide_is_older_than_the_last_scheduled_time() {
        let times = [4 * 60];
        let now = at(29, 10, 0);
        let four_am = at(29, 4, 0).timestamp();

        assert!(update_due(&times, now, None)); // nothing written yet
        assert!(update_due(&times, now, Some(four_am - 3600))); // last written before 04:00: due
        assert!(!update_due(&times, now, Some(four_am + 60))); // written after 04:00: current
        assert!(!update_due(&[], now, None)); // no schedule, never due

        // a failed attempt counts as done, so it is not retried every 30 seconds
        let attempt = at(29, 4, 1).timestamp();
        assert!(!update_due(&times, at(29, 4, 2), Some(attempt)));
        // and tomorrow's 04:00 is due again
        assert!(update_due(&times, at(30, 4, 0), Some(attempt)));
    }
}
