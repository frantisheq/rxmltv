//! Notifications about programmes: the rules ("tell me when X is on"), what has already been announced, the
//! check that decides what is due, and sending through Apprise (https://appriseit.com/).
//!
//! The same code runs in the terminal UI (popup, sound, Apprise) and in the service mode (Apprise only), so a
//! notification is *claimed* in the "notified" file before it is sent: whoever claims it sends it, the other
//! one skips it. A rule is kept in `rxmltv.notifications`, what was announced in `rxmltv.notified`.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

use crate::guide::Guide;
use crate::store::{read_lines, title_key, write_lines};
use crate::t;

/// How long after its start a programme can still be announced.
const GRACE_SECONDS: i64 = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    /// Only the first time the programme shows up.
    Once,
    /// Every time it shows up in the guide.
    Always,
}

impl Repeat {
    pub fn code(self) -> &'static str {
        match self {
            Repeat::Once => "once",
            Repeat::Always => "always",
        }
    }

    pub fn parse(text: &str) -> Option<Repeat> {
        match text.trim().to_ascii_lowercase().as_str() {
            "once" => Some(Repeat::Once),
            "always" => Some(Repeat::Always),
            _ => None,
        }
    }

    pub fn flip(self) -> Repeat {
        match self {
            Repeat::Once => Repeat::Always,
            Repeat::Always => Repeat::Once,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Once => t!("once"),
            Repeat::Always => t!("every time"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// Only on the channel the rule was made on.
    ThisChannel,
    AllChannels,
}

impl Scope {
    pub fn code(self) -> &'static str {
        match self {
            Scope::ThisChannel => "channel",
            Scope::AllChannels => "all",
        }
    }

    pub fn parse(text: &str) -> Option<Scope> {
        match text.trim().to_ascii_lowercase().as_str() {
            "channel" => Some(Scope::ThisChannel),
            "all" => Some(Scope::AllChannels),
            _ => None,
        }
    }

    pub fn flip(self) -> Scope {
        match self {
            Scope::ThisChannel => Scope::AllChannels,
            Scope::AllChannels => Scope::ThisChannel,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::ThisChannel => t!("this channel"),
            Scope::AllChannels => t!("all channels"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: u64,
    pub title: String,
    /// The title as programmes are matched by it: lower case, no diacritics.
    pub key: String,
    pub repeat: Repeat,
    pub scope: Scope,
    /// The XMLTV id of the channel (matters for `Scope::ThisChannel`).
    pub channel_id: String,
    pub channel_name: String,
    /// Minutes before the start; None: the default from the settings.
    pub minutes: Option<u32>,
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ").trim().to_string()
}

impl Rule {
    fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.id,
            self.repeat.code(),
            self.scope.code(),
            self.minutes.map(|m| m.to_string()).unwrap_or_default(),
            clean(&self.channel_id),
            clean(&self.channel_name),
            clean(&self.title)
        )
    }

    fn from_line(line: &str) -> Option<Rule> {
        let mut parts = line.splitn(7, '\t');
        let id = parts.next()?.trim().parse().ok()?;
        let repeat = Repeat::parse(parts.next()?)?;
        let scope = Scope::parse(parts.next()?)?;
        let minutes = parts.next()?.trim().parse::<u32>().ok();
        let channel_id = parts.next()?.trim().to_string();
        let channel_name = parts.next()?.trim().to_string();
        let title = parts.next()?.trim().to_string();
        let key = title_key(&title);
        if key.is_empty() {
            return None;
        }
        Some(Rule { id, title, key, repeat, scope, channel_id, channel_name, minutes })
    }
}

pub struct Rules {
    rules: Vec<Rule>,
    path: PathBuf,
}

impl Rules {
    pub fn load(path: PathBuf) -> Rules {
        let mut rules = Rules { rules: Vec::new(), path };
        rules.reload();
        rules
    }

    /// Reads the file again (the terminal UI and the service share it).
    pub fn reload(&mut self) {
        self.rules = read_lines(&self.path).iter().filter_map(|l| Rule::from_line(l)).collect();
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn save(&self) -> Result<(), String> {
        let lines: Vec<String> = self.rules.iter().map(Rule::to_line).collect();
        write_lines(&self.path, &lines)
            .map_err(|e| t!("Could not save {path}: {error}", path = self.path.display(), error = e))
    }

    fn next_id(&self) -> u64 {
        self.rules.iter().map(|r| r.id).max().unwrap_or(0) + 1
    }

    pub fn add(
        &mut self,
        title: &str,
        repeat: Repeat,
        scope: Scope,
        channel_id: &str,
        channel_name: &str,
        minutes: Option<u32>,
    ) -> Result<u64, String> {
        let title = clean(title);
        let key = title_key(&title);
        if key.is_empty() {
            return Err(t!("A notification needs a title.").to_string());
        }
        let id = self.next_id();
        self.rules.push(Rule {
            id,
            title,
            key,
            repeat,
            scope,
            channel_id: clean(channel_id),
            channel_name: clean(channel_name),
            minutes,
        });
        self.save()?;
        Ok(id)
    }

    /// The rule that covers this programme on this channel, if any.
    pub fn find_for(&self, key: &str, channel_id: &str) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|r| r.key == key && (r.scope == Scope::AllChannels || r.channel_id == channel_id))
    }

    pub fn matches(&self, key: &str, channel_id: &str) -> bool {
        self.find_for(key, channel_id).is_some()
    }

    /// Removes the rule that covers the programme, or adds one. Ok(true): it was added.
    pub fn toggle(
        &mut self,
        title: &str,
        channel_id: &str,
        channel_name: &str,
        repeat: Repeat,
        scope: Scope,
    ) -> Result<bool, String> {
        let key = title_key(title);
        if let Some(id) = self.find_for(&key, channel_id).map(|r| r.id) {
            self.remove(id)?;
            return Ok(false);
        }
        self.add(title, repeat, scope, channel_id, channel_name, None).map(|_| true)
    }

    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        self.rules.retain(|r| r.id != id);
        self.save()
    }

    pub fn update(&mut self, id: u64, change: impl FnOnce(&mut Rule)) -> Result<(), String> {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            change(rule);
        }
        self.save()
    }
}

/// Which airings have been announced: (rule, channel, start time).
pub struct Fired {
    entries: HashSet<(u64, String, i64)>,
    path: PathBuf,
}

impl Fired {
    pub fn load(path: PathBuf) -> Fired {
        let mut fired = Fired { entries: HashSet::new(), path };
        fired.reload();
        fired
    }

    pub fn reload(&mut self) {
        self.entries = read_lines(&self.path)
            .iter()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                Some((parts.next()?.trim().parse().ok()?, parts.next()?.to_string(), parts.next()?.trim().parse().ok()?))
            })
            .collect();
    }

    pub fn contains(&self, rule: u64, channel: &str, start: i64) -> bool {
        self.entries.contains(&(rule, channel.to_string(), start))
    }

    /// The airings of a rule that were announced.
    pub fn for_rule(&self, rule: u64) -> Vec<(String, i64)> {
        self.entries.iter().filter(|(r, _, _)| *r == rule).map(|(_, c, s)| (c.clone(), *s)).collect()
    }

    fn save(&self) -> Result<(), String> {
        let mut lines: Vec<String> = self.entries.iter().map(|(r, c, s)| format!("{r}\t{c}\t{s}")).collect();
        lines.sort();
        write_lines(&self.path, &lines)
            .map_err(|e| t!("Could not save {path}: {error}", path = self.path.display(), error = e))
    }

    /// Remembers an announcement. Reads the file first, so that two programs claiming at the same moment
    /// rarely both win.
    pub fn claim(&mut self, due: &Due) -> Result<bool, String> {
        self.reload();
        if self.contains(due.rule_id, &due.channel_id, due.start) {
            return Ok(false);
        }
        self.entries.insert((due.rule_id, due.channel_id.clone(), due.start));
        self.save()?;
        Ok(true)
    }

    /// Takes a claim back (the sending failed, so the next check tries again).
    pub fn unclaim(&mut self, due_key: &str) {
        self.reload();
        self.entries.retain(|(r, c, s)| format!("{r}|{c}|{s}") != due_key);
        let _ = self.save();
    }

    /// Forgets what was announced for a rule ("notify once" can then notify again).
    pub fn clear_rule(&mut self, rule: u64) -> Result<(), String> {
        self.reload();
        self.entries.retain(|(r, _, _)| *r != rule);
        self.save()
    }

    /// Forgets announcements of programmes that started before `before` (a timestamp).
    pub fn prune(&mut self, before: i64) {
        self.reload();
        let had = self.entries.len();
        self.entries.retain(|(_, _, start)| *start >= before);
        if self.entries.len() != had {
            let _ = self.save();
        }
    }
}

/// A programme that is due for a notification.
#[derive(Clone, Debug)]
pub struct Due {
    pub rule_id: u64,
    pub title: String,
    pub channel_id: String,
    pub channel_name: String,
    /// Start time as a timestamp, and as "20:15" (the guide's own clock).
    pub start: i64,
    pub time: String,
    pub minutes_left: i64,
    /// Somebody (the service, the terminal UI) has already announced it.
    pub claimed: bool,
}

impl Due {
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.rule_id, self.channel_id, self.start)
    }

    /// (title, body) of the notification.
    pub fn message(&self) -> (String, String) {
        let body = if self.minutes_left > 0 {
            t!(
                "Starts in {minutes} min on {channel} at {time}",
                minutes = self.minutes_left,
                channel = self.channel_name,
                time = self.time
            )
        } else {
            t!("Starting now on {channel} at {time}", channel = self.channel_name, time = self.time)
        };
        (self.title.clone(), body)
    }
}

/// What is due at `now`: programmes of a rule that start within their lead time (or just started). A "once" rule
/// yields its first airing only, and, once that was announced, only that one (so a popup can still show it).
pub fn due(rules: &Rules, fired: &Fired, guide: &Guide, now: i64, default_minutes: u32) -> Vec<Due> {
    let mut out: Vec<Due> = Vec::new();

    for rule in rules.rules() {
        let lead = i64::from(rule.minutes.unwrap_or(default_minutes)) * 60;
        let announced_once = rule.repeat == Repeat::Once && !fired.for_rule(rule.id).is_empty();

        for &(c, p) in guide.airings(&rule.key) {
            let channel = &guide.channels[c];
            if rule.scope == Scope::ThisChannel && channel.id != rule.channel_id {
                continue;
            }
            let programme = &channel.programmes[p];
            let start = programme.start.timestamp();
            if now < start - lead || now >= start + GRACE_SECONDS {
                continue;
            }
            let claimed = fired.contains(rule.id, &channel.id, start);
            if announced_once && !claimed {
                continue; // the one airing of this rule was announced already
            }
            out.push(Due {
                rule_id: rule.id,
                title: programme.title.clone(),
                channel_id: channel.id.clone(),
                channel_name: channel.name.clone(),
                start,
                time: programme.start.format("%H:%M").to_string(),
                minutes_left: if start > now { (start - now + 59) / 60 } else { 0 },
                claimed,
            });
            if rule.repeat == Repeat::Once {
                break;
            }
        }
    }

    out.sort_by_key(|d| d.start);
    out
}

// ---------- Apprise ----------

/// The notification URLs, separated by spaces, commas or new lines.
pub fn split_urls(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Sends a notification with the `apprise` program. `command` may carry options (`apprise --tag family`,
/// `apprise --config /etc/apprise.yml`); the URLs may be empty when Apprise has its own configuration.
pub async fn apprise(command: &str, urls: &str, title: &str, body: &str) -> Result<(), String> {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else { return Err(t!("The Apprise command is empty.").to_string()) };

    let child = Command::new(program)
        .args(words)
        .arg("-t")
        .arg(title)
        .arg("-b")
        .arg(body)
        .args(split_urls(urls))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                t!("The Apprise program was not found. Install it (see https://appriseit.com/) or set its path.")
                    .to_string()
            } else {
                t!("Cannot start Apprise: {error}", error = e)
            }
        })?;

    let output = child.wait_with_output().await.map_err(|e| t!("Cannot wait for Apprise: {error}", error = e))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    Err(if last.is_empty() { t!("Apprise stopped with {status}.", status = output.status) } else { last })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rxmltv-test-{}-notify-{name}", std::process::id()))
    }

    /// A guide with "Nautilus" on CT1 at 20:00 and 22:00 (Prague time, UTC+1) on 2026-10-29, and on Nova at 20:00.
    fn guide() -> Guide {
        let xml = r#"<tv>
            <channel id="cz-ct1-2"><display-name>ČT1</display-name></channel>
            <channel id="cz-nova-5"><display-name>Nova</display-name></channel>
            <programme start="20261029200000 +0100" stop="20261029210000 +0100" channel="cz-ct1-2"><title>Nautilus</title></programme>
            <programme start="20261029220000 +0100" stop="20261029230000 +0100" channel="cz-ct1-2"><title>Nautilus</title></programme>
            <programme start="20261029200000 +0100" stop="20261029210000 +0100" channel="cz-nova-5"><title>Nautilus</title></programme>
            <programme start="20261029200000 +0100" stop="20261029210000 +0100" channel="cz-nova-5"><title>Zprávy</title></programme>
        </tv>"#;
        crate::guide::parse(xml).unwrap()
    }

    fn start() -> i64 {
        // 20:00 +0100
        chrono::DateTime::parse_from_str("20261029200000 +0100", "%Y%m%d%H%M%S %z").unwrap().timestamp()
    }

    #[test]
    fn rules_are_saved_and_loaded() {
        let path = temp("rules");
        let _ = std::fs::remove_file(&path);
        let mut rules = Rules::load(path.clone());
        let a = rules.add("Večer s hvězdami", Repeat::Once, Scope::ThisChannel, "cz-ct1-2", "ČT1", None).unwrap();
        let b = rules.add("Nautilus", Repeat::Always, Scope::AllChannels, "cz-nova-5", "Nova", Some(10)).unwrap();
        assert_ne!(a, b);
        assert!(rules.add("   ", Repeat::Once, Scope::AllChannels, "", "", None).is_err());

        let again = Rules::load(path.clone());
        assert_eq!(again.rules().len(), 2);
        assert_eq!(again.rules()[1].minutes, Some(10));
        assert_eq!(again.rules()[0].channel_name, "ČT1");
        assert_eq!(again.rules()[1].repeat, Repeat::Always);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rules_cover_a_channel_or_all() {
        let path = temp("cover");
        let _ = std::fs::remove_file(&path);
        let mut rules = Rules::load(path.clone());
        rules.add("Nautilus", Repeat::Once, Scope::ThisChannel, "cz-ct1-2", "ČT1", None).unwrap();
        assert!(rules.matches("nautilus", "cz-ct1-2"));
        assert!(!rules.matches("nautilus", "cz-nova-5"));
        rules.add("Zprávy", Repeat::Once, Scope::AllChannels, "cz-ct1-2", "ČT1", None).unwrap();
        assert!(rules.matches("zpravy", "anything"));

        // toggling removes an existing rule and adds a missing one
        assert_eq!(rules.toggle("Nautilus", "cz-ct1-2", "ČT1", Repeat::Once, Scope::ThisChannel).unwrap(), false);
        assert!(!rules.matches("nautilus", "cz-ct1-2"));
        assert_eq!(rules.toggle("Nautilus", "cz-ct1-2", "ČT1", Repeat::Always, Scope::AllChannels).unwrap(), true);
        assert_eq!(rules.find_for("nautilus", "cz-nova-5").unwrap().repeat, Repeat::Always);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_programme_is_due_in_its_window() {
        let rules_path = temp("window-rules");
        let fired_path = temp("window-fired");
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
        let mut rules = Rules::load(rules_path.clone());
        rules.add("Zprávy", Repeat::Once, Scope::AllChannels, "", "", None).unwrap();
        let fired = Fired::load(fired_path.clone());
        let g = guide();
        let s = start();

        assert!(due(&rules, &fired, &g, s - 6 * 60, 5).is_empty()); // more than 5 minutes before
        let soon = due(&rules, &fired, &g, s - 4 * 60, 5);
        assert_eq!(soon.len(), 1);
        assert_eq!(soon[0].channel_name, "Nova");
        assert_eq!(soon[0].time, "20:00");
        assert_eq!(soon[0].minutes_left, 4);
        assert_eq!(due(&rules, &fired, &g, s + 30, 5).len(), 1); // just started: still announced
        assert!(due(&rules, &fired, &g, s + 61, 5).is_empty()); // too late

        // the rule's own minutes beat the default
        let id = rules.rules()[0].id;
        rules.update(id, |r| r.minutes = Some(30)).unwrap();
        assert_eq!(due(&rules, &fired, &g, s - 25 * 60, 5).len(), 1);
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
    }

    #[test]
    fn a_once_rule_announces_one_airing_only() {
        let rules_path = temp("once-rules");
        let fired_path = temp("once-fired");
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
        let mut rules = Rules::load(rules_path.clone());
        rules.add("Nautilus", Repeat::Once, Scope::AllChannels, "", "", None).unwrap();
        let mut fired = Fired::load(fired_path.clone());
        let g = guide();
        let s = start();

        // 20:00 is on CT1 and on Nova: one is picked
        let first = due(&rules, &fired, &g, s - 120, 5);
        assert_eq!(first.len(), 1);
        assert!(!first[0].claimed);
        assert!(fired.claim(&first[0]).unwrap());
        assert!(!fired.claim(&first[0]).unwrap()); // a second claim loses

        // announced: the same airing still shows (claimed), the 22:00 one does not
        let again = due(&rules, &fired, &g, s - 120, 5);
        assert_eq!(again.len(), 1);
        assert!(again[0].claimed);
        assert!(due(&rules, &fired, &g, s + 2 * 3600 - 120, 5).is_empty());

        // re-arming forgets the announcement
        fired.clear_rule(rules.rules()[0].id).unwrap();
        assert!(!due(&rules, &fired, &g, s + 2 * 3600 - 120, 5).is_empty());
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
    }

    #[test]
    fn an_always_rule_announces_every_airing_once() {
        let rules_path = temp("always-rules");
        let fired_path = temp("always-fired");
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
        let mut rules = Rules::load(rules_path.clone());
        rules.add("Nautilus", Repeat::Always, Scope::ThisChannel, "cz-ct1-2", "ČT1", None).unwrap();
        let mut fired = Fired::load(fired_path.clone());
        let g = guide();
        let s = start();

        let evening = due(&rules, &fired, &g, s - 120, 5);
        assert_eq!(evening.len(), 1); // CT1 only: the rule is for this channel
        assert_eq!(evening[0].channel_id, "cz-ct1-2");
        fired.claim(&evening[0]).unwrap();

        let late = due(&rules, &fired, &g, s + 2 * 3600 - 120, 5);
        assert_eq!(late.len(), 1);
        assert!(!late[0].claimed); // the 22:00 airing is new

        // announcements of long gone programmes are forgotten
        fired.prune(s + 1);
        assert!(fired.for_rule(rules.rules()[0].id).is_empty());
        let _ = std::fs::remove_file(&rules_path);
        let _ = std::fs::remove_file(&fired_path);
    }

    #[test]
    fn messages_say_when() {
        let mut d = Due {
            rule_id: 1,
            title: "Nautilus".into(),
            channel_id: "cz-ct1-2".into(),
            channel_name: "ČT1".into(),
            start: 0,
            time: "20:00".into(),
            minutes_left: 5,
            claimed: false,
        };
        let (title, body) = d.message();
        assert_eq!(title, "Nautilus");
        assert_eq!(body, "Starts in 5 min on ČT1 at 20:00");
        d.minutes_left = 0;
        assert_eq!(d.message().1, "Starting now on ČT1 at 20:00");
    }

    #[test]
    fn urls_are_split() {
        assert_eq!(split_urls("tgram://a/b, mailto://x\nslack://y  "), vec!["tgram://a/b", "mailto://x", "slack://y"]);
        assert!(split_urls("  ").is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apprise_runs_the_command() {
        // `true` accepts any arguments and succeeds, `false` fails
        assert!(apprise("true", "json://localhost", "Title", "Body").await.is_ok());
        assert!(apprise("false", "", "Title", "Body").await.is_err());
        assert!(apprise("  ", "", "Title", "Body").await.is_err());
        assert!(apprise("rxmltv-no-such-program", "", "Title", "Body").await.is_err());
    }
}
