//! Translations of the app's texts (English, Czech, Slovak).
//!
//! The English text itself is the key: `t!("Saved the settings to {path}", path = p)` looks the text up in the
//! table in `i18n_table.rs`, and falls back to the English text when a language has no translation. Names in
//! braces are filled in after translating.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

include!("i18n_table.rs");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Cs,
    Sk,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Cs, Lang::Sk];

    /// The code used in the settings file and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Cs => "cs",
            Lang::Sk => "sk",
        }
    }

    /// The language's name in that language.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Cs => "Čeština",
            Lang::Sk => "Slovenčina",
        }
    }

    pub fn parse(text: &str) -> Option<Lang> {
        let t = text.trim().to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == t || l.name().to_lowercase() == t)
    }

    pub fn step(self, delta: i32) -> Lang {
        let at = Lang::ALL.iter().position(|l| *l == self).unwrap_or(0) as i32;
        Lang::ALL[(at + delta).rem_euclid(Lang::ALL.len() as i32) as usize]
    }

    /// The language of the user's environment (LC_ALL, LC_MESSAGES, LANG), English if it is neither Czech nor Slovak.
    pub fn detect() -> Lang {
        for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(name) {
                let value = value.to_ascii_lowercase();
                if value.starts_with("cs") {
                    return Lang::Cs;
                }
                if value.starts_with("sk") {
                    return Lang::Sk;
                }
                if !value.is_empty() {
                    return Lang::En;
                }
            }
        }
        Lang::En
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Cs,
        2 => Lang::Sk,
        _ => Lang::En,
    }
}

fn table() -> &'static HashMap<&'static str, (&'static str, &'static str)> {
    static MAP: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();
    MAP.get_or_init(|| TABLE.iter().map(|(en, cs, sk)| (*en, (*cs, *sk))).collect())
}

/// The text in the given language (the English text when there is no translation).
pub fn tr_in(lang: Lang, en: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Cs => table().get(en).map(|t| t.0).unwrap_or(en),
        Lang::Sk => table().get(en).map(|t| t.1).unwrap_or(en),
    }
}

/// The text in the current language.
pub fn tr(en: &'static str) -> &'static str {
    tr_in(get(), en)
}

/// Like `tr`, then each `{name}` is replaced by its value.
pub fn tr_args(en: &'static str, args: &[(&str, String)]) -> String {
    let mut text = tr(en).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// Short weekday name in the current language ("Thu", "Čt", "Št").
pub fn weekday_short(day: chrono::Weekday) -> &'static str {
    weekday_short_in(get(), day)
}

pub fn weekday_short_in(lang: Lang, day: chrono::Weekday) -> &'static str {
    const EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const CS: [&str; 7] = ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"];
    const SK: [&str; 7] = ["Po", "Ut", "St", "Št", "Pi", "So", "Ne"];
    let i = day.num_days_from_monday() as usize;
    match lang {
        Lang::En => EN[i],
        Lang::Cs => CS[i],
        Lang::Sk => SK[i],
    }
}

/// Short month name in the current language; `month` is 1 to 12.
pub fn month_short(month: u32) -> &'static str {
    month_short_in(get(), month)
}

pub fn month_short_in(lang: Lang, month: u32) -> &'static str {
    const EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const CS: [&str; 12] = ["led", "úno", "bře", "dub", "kvě", "čvn", "čvc", "srp", "zář", "říj", "lis", "pro"];
    const SK: [&str; 12] = ["jan", "feb", "mar", "apr", "máj", "jún", "júl", "aug", "sep", "okt", "nov", "dec"];
    let i = (month.clamp(1, 12) - 1) as usize;
    match lang {
        Lang::En => EN[i],
        Lang::Cs => CS[i],
        Lang::Sk => SK[i],
    }
}

/// "Thu 29.10. 20:15" in the current language.
pub fn when_short<T: chrono::Datelike + chrono::Timelike>(t: &T) -> String {
    when_short_in(get(), t)
}

pub fn when_short_in<T: chrono::Datelike + chrono::Timelike>(lang: Lang, t: &T) -> String {
    format!("{} {:02}.{:02}. {:02}:{:02}", weekday_short_in(lang, t.weekday()), t.day(), t.month(), t.hour(), t.minute())
}

/// `t!("text")` is the text in the current language; `t!("text {a}", a = x)` fills in `{a}`.
#[macro_export]
macro_rules! t {
    ($text:literal) => {
        $crate::i18n::tr($text)
    };
    ($text:literal, $($key:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr_args($text, &[$((stringify!($key), ($value).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names in braces, sorted.
    fn placeholders(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let Some(end) = rest[start..].find('}') else { break };
            found.push(rest[start + 1..start + end].to_string());
            rest = &rest[start + end + 1..];
        }
        found.sort();
        found
    }

    #[test]
    fn every_translation_keeps_the_placeholders() {
        for (en, cs, sk) in TABLE {
            assert_eq!(placeholders(en), placeholders(cs), "Czech: {en}");
            assert_eq!(placeholders(en), placeholders(sk), "Slovak: {en}");
            assert!(!cs.is_empty() && !sk.is_empty(), "empty translation: {en}");
        }
    }

    /// Every `t!("...")` in the source must have a row in the table, so no text silently stays English.
    #[test]
    fn every_text_in_the_code_has_a_translation() {
        fn visit(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit(&path, files);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        visit(std::path::Path::new("src"), &mut files);

        let known: std::collections::HashSet<&str> = TABLE.iter().map(|(en, _, _)| *en).collect();
        let mut missing: Vec<String> = Vec::new();
        for file in files {
            let name = file.to_string_lossy().to_string();
            if name.ends_with("i18n.rs") || name.ends_with("i18n_table.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let mut rest = text.as_str();
            let mut offset = 0; // how far into `text` `rest` starts
            while let Some(at) = rest.find("t!(\"") {
                // only the macro `t!`, not the end of another name such as `format!` or `assert!`
                let before = text[..offset + at].chars().last();
                let is_ours = !before.is_some_and(|c| c.is_alphanumeric() || c == '_');
                rest = &rest[at + 4..];
                offset += at + 4;
                if !is_ours {
                    continue;
                }
                // the literal ends at the first quote that is not escaped
                let mut literal = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('"') => literal.push('"'),
                            Some('\\') => literal.push('\\'),
                            Some(other) => {
                                literal.push('\\');
                                literal.push(other);
                            }
                            None => break,
                        },
                        '"' => break,
                        other => literal.push(other),
                    }
                }
                if !known.contains(literal.as_str()) {
                    missing.push(format!("{name}: {literal}"));
                }
            }
        }
        assert!(missing.is_empty(), "texts without a translation:\n{}", missing.join("\n"));
    }

    #[test]
    fn no_text_is_listed_twice() {
        let mut seen = std::collections::HashSet::new();
        for (en, _, _) in TABLE {
            assert!(seen.insert(*en), "listed twice: {en}");
        }
    }

    #[test]
    fn days_and_months_are_named_in_every_language() {
        use chrono::TimeZone;
        let when = chrono::FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 10, 29, 20, 15, 0).unwrap();
        assert_eq!(when_short_in(Lang::En, &when), "Thu 29.10. 20:15");
        assert_eq!(month_short_in(Lang::En, 10), "Oct");
        assert_eq!(when_short_in(Lang::Cs, &when), "Čt 29.10. 20:15");
        assert_eq!(month_short_in(Lang::Cs, 10), "říj");
        assert_eq!(when_short_in(Lang::Sk, &when), "Št 29.10. 20:15");
        assert_eq!(month_short_in(Lang::Sk, 5), "máj");
    }

    #[test]
    fn texts_are_translated_and_fall_back_to_english() {
        assert_eq!(tr_in(Lang::En, "Guide"), "Guide");
        assert_eq!(tr_in(Lang::Cs, "Favorites"), "Oblíbené");
        assert_eq!(tr_in(Lang::Sk, "Favorites"), "Obľúbené");
        assert_eq!(tr_in(Lang::Cs, "A text nobody translated"), "A text nobody translated");
    }

    #[test]
    fn languages_parse_and_cycle() {
        assert_eq!(Lang::parse("CS"), Some(Lang::Cs));
        assert_eq!(Lang::parse("slovenčina"), Some(Lang::Sk));
        assert_eq!(Lang::parse("de"), None);
        assert_eq!(Lang::En.step(-1), Lang::Sk);
        assert_eq!(Lang::Sk.step(1), Lang::En);
    }
}
