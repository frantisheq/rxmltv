//! Combines the structured fields of a detail file with what can be parsed from the title.
//! Free-text parsing of the description is only a fallback for programmes without structured data
//! (typically foreign channels).

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{Datelike, Utc};
use regex::{Captures, Regex, RegexBuilder};
use roxmltree::Node;

use crate::util::{
    blank, child, child_value, count_letters, eq_ignore_case, html_decode, is_el, key, opt_blank, value,
};

// =====================================================================
//  Data
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeInfo {
    pub season: Option<u32>,
    pub episode: u32,
    pub total: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Category {
    pub lang: &'static str,
    pub text: String,
}

/// The real fields of a detail file (v5-porad.php), read without any text parsing.
#[derive(Debug, Default)]
pub struct DetailInfo {
    pub original_title: Option<String>, // <b>   episode title for series, original title for films
    pub kind: Option<String>,           // <i><t>   Film, Seriál, Dokument, Zábava, Dětem, Sport
    pub tags: Vec<String>,              // <i><st><tt>   komedie, krimi, ...
    pub directors: Vec<Person>,         // <i><l><o t="r">
    pub writers: Vec<Person>,           // t="s"
    pub producers: Vec<Person>,         // t="p"
    pub composers: Vec<Person>,         // t="m"
    pub actors: Vec<Person>,            // t="h" (with role)
    pub country: Option<String>,        // <i><z>
    pub year: Option<i32>,              // <i><r>
    pub minutes: Option<u32>,           // <i><d>
    pub rating: Option<String>,         // <h>  (e.g. "6.3")
    pub icon: Option<String>,           // first programme image
    pub flags: HashSet<String>,         // <qq><q id="..."> codes, see `extract`
}

/// A parsed detail file, copied out of the XML so it can be kept across `await` points.
#[derive(Debug)]
pub struct DetailData {
    pub start: Option<String>,      // <s o="...">
    pub stop: Option<String>,       // <s d="...">
    pub title: Option<String>,      // <n>
    pub short_desc: Option<String>, // <p><k>
    pub long_desc: Option<String>,  // <p><d>
    pub info: DetailInfo,
}

impl DetailData {
    pub fn from_node(root: Node) -> Self {
        let s = child(root, "s");
        let p = child(root, "p");
        DetailData {
            start: s.and_then(|s| s.attribute("o")).map(String::from),
            stop: s.and_then(|s| s.attribute("d")).map(String::from),
            title: child_value(root, "n"),
            short_desc: p.and_then(|p| child_value(p, "k")),
            long_desc: p.and_then(|p| child_value(p, "d")),
            info: read_detail(root),
        }
    }
}

/// Everything the XMLTV writer needs for one programme.
#[derive(Debug)]
pub struct ProgrammeInfo {
    pub title: String,
    pub raw_title: String,
    pub sub_title: Option<String>,
    pub original_title: Option<String>,
    pub description: Option<String>,
    pub episode: Option<EpisodeInfo>,
    pub year: Option<i32>,
    pub country: Option<String>,
    pub length_minutes: Option<u32>,
    pub directors: Vec<Person>,
    pub actors: Vec<Person>,
    pub writers: Vec<Person>,
    pub producers: Vec<Person>,
    pub composers: Vec<Person>,
    pub presenters: Vec<Person>,
    pub categories: Vec<Category>,
    pub rating: Option<String>,
    pub icon: Option<String>,
    pub quality: Option<&'static str>,
    pub aspect: Option<&'static str>,
    pub black_and_white: bool,
    pub audio: Option<&'static str>,
    pub subtitle_types: Vec<&'static str>,
    pub premiere: bool,
    pub previously_shown: bool,
}

// =====================================================================
//  Regular expressions
// =====================================================================

// Built once, on first use. The `regex` crate has no look-around, so the three patterns that
// needed it in the C# version are handled in code instead (see `find_credit`, `split_sentences`
// and PREMIERE_RX).
fn rx(pattern: &str) -> Regex {
    // Name patterns repeat large Unicode classes, so raise the compiled-size limit.
    RegexBuilder::new(pattern)
        .size_limit(512 << 20)
        .build()
        .unwrap_or_else(|e| panic!("invalid regex {pattern}: {e}"))
}

fn rx_i(pattern: &str) -> Regex {
    rx(&format!("(?i){pattern}"))
}

// The day listing gives the clean series name in <n u="...">; whatever follows it is the marker.
static SUFFIX_RX: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"^\s*(?:(?P<roman>[IVX]{1,6})\s*)?\((?P<ep>[0-9]{1,5})(?:/(?P<tot>[0-9]{1,5}))?\)\s*$")
});

// Heuristic for when no clean title is available.
static TITLE_EPISODE_RX: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"^(?P<name>.*?)(?:\s+(?P<roman>[IVX]{1,6}))?\s*\((?P<ep>[0-9]{1,5})(?:/(?P<tot>[0-9]{1,5}))?\)\s*$")
});

static ROMAN_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"^X{0,3}(?:IX|IV|V?I{0,3})$"));

static ORIGINAL_TITLE_PREFIX_RX: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?s)^\((?P<t>[^()]{2,100})\)\s*(?P<rest>.*)$"));
static DATE_LIKE_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"\d{1,2}\.\s?\d{1,2}\."));
static COUNTRY_YEAR_RX: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^[\p{L} .'-]{2,30},\s*(?:19|20)\d\d$"));
static FLAG_WORD_RX: LazyLock<Regex> =
    LazyLock::new(|| rx_i(r"^(?:premiéra|repríza|premiere|live|živě)$"));
static TRAILING_ROMAN_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"\s+(?P<r>[IVX]{1,6})$"));

static YEAR_RX: LazyLock<Regex> =
    LazyLock::new(|| rx(r"\((?:\p{Lu}[\p{L} .'-]{1,30},\s*)?(?P<year>(?:19|20)[0-9][0-9])\)"));

const PARTICLE: &str = "(?:de|da|di|del|della|van|von|der|den|ter|ten|la|le|du|dos|das|el|al|bin|ibn)";
const NAME_INITIALS: &str = r"(?:\p{Lu}\p{Ll}?\.\s?)+\p{Lu}[\p{L}'’-]+"; // "D. Svátek", "Ph. Williams"
const SEP: &str = r"(?:\s*,\s*|\s+(?:a|und|and|i|és|et|y)\s+|\s*&\s*|\s*/\s*)";

/// One person's name, e.g. "Jiří Bartoška".
static NAME_PATTERN: LazyLock<String> = LazyLock::new(|| {
    let name_full = String::new()
        + r"\p{Lu}[\p{L}'’-]+(?:\s+(?:"
        + PARTICLE
        + r"\s+){0,2}\p{Lu}[\p{L}'’-]+){0,2}";
    String::new() + "(?:" + NAME_INITIALS + "|" + &name_full + ")"
});

static NAME_LIST_PATTERN: LazyLock<String> = LazyLock::new(|| {
    let name: &str = &NAME_PATTERN;
    String::new() + name + "(?:" + SEP + name + ")*"
});

static NAME_RX: LazyLock<Regex> = LazyLock::new(|| rx(&NAME_PATTERN));

/// `optional`: the label may be followed by a plain space ("Hrají Roy Garber");
/// `required`: it needs a colon. The match must not start in the middle of a word
/// (the C# version used a look-behind for that, `find_credit` does it by hand).
fn credit(optional: &str, required: &str) -> Regex {
    let list: &str = &NAME_LIST_PATTERN;
    let pattern = String::new()
        + r"(?:(?:"
        + optional
        + r")(?:\s*:\s*|\s+)|(?:"
        + required
        + r")\s*:\s*)(?P<list>"
        + list
        + ")";
    rx(&pattern)
}

static DIRECTOR_RX: LazyLock<Regex> = LazyLock::new(|| {
    credit(
        "Režie|Réžia|Reżyseria",
        "Rendező|Regie|Réalisateurs?|Reżyser|Director|Directed by",
    )
});

static ACTOR_RX: LazyLock<Regex> = LazyLock::new(|| {
    credit(
        "Hrají|V rolích|Účinkují|Účinkuje|Účinkujú|W rolach",
        "Főszereplők|Szereplők|Darstellers?|Cast|Starring|Obsada",
    )
});

static PRESENTER_RX: LazyLock<Regex> = LazyLock::new(|| {
    credit(
        "Moderuje|Moderují|Moderátor|Moderátorka|Prowadzi|Vedie",
        "Moderation|Moderiert von|Presented by|Présentation|Műsorvezető",
    )
});

static EPISODE_PLACEHOLDER_RX: LazyLock<Regex> =
    LazyLock::new(|| rx_i(r"^Episode\s*#\s*[0-9]+(?:\.[0-9]+)?$"));

// "(Premiéra)" anywhere, or a trailing "Premiéra" as its own sentence.
static PREMIERE_RX: LazyLock<Regex> =
    LazyLock::new(|| rx(r"\(Premiéra\)|[.!?]\sPremiéra\s*\.?\s*$"));
static RERUN_RX: LazyLock<Regex> = LazyLock::new(|| rx_i(r"\(repríza\)"));

static HTML_TAG_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"</?[A-Za-z][^>]*>"));

// The API puts a rating in front of many descriptions: "86% Britský seriál podle ...".
static LEADING_PERCENT_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"^\s*\d{1,3}\s?%\s*"));
static SPACES_RX: LazyLock<Regex> = LazyLock::new(|| rx(r"\s{2,}"));

// =====================================================================
//  Structured detail file
// =====================================================================

fn type_map(kind: &str) -> Option<&'static str> {
    match kind.to_lowercase().as_str() {
        "film" => Some("Movie"),
        "seriál" => Some("Series"),
        "dokument" => Some("Documentary"),
        "sport" => Some("Sports"),
        "dětem" => Some("Children"),
        "zábava" => Some("Entertainment"),
        _ => None,
    }
}

/// Digits only (no sign, no spaces), within [min, max].
fn parse_int(s: Option<&str>, min: i64, max: i64) -> Option<i64> {
    let s = s?;
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    if n >= min && n <= max {
        Some(n)
    } else {
        None
    }
}

/// "6.3" -> "6.3", "7.0" -> "7", "6.25" -> "6.3" (like .NET's "0.#"); anything outside 0..=10 is dropped.
/// The comma must already have been turned into a point.
fn parse_rating(raw: &str) -> Option<String> {
    let (int_part, frac_part) = match raw.split_once('.') {
        Some((i, f)) => (i, f),
        None => (raw, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) || !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let int_value: u64 = if int_part.is_empty() { 0 } else { int_part.parse().ok()? };
    if int_value > 10 || (int_value == 10 && frac_part.chars().any(|c| c != '0')) {
        return None;
    }

    let digit = |i: usize| frac_part.chars().nth(i).and_then(|c| c.to_digit(10)).unwrap_or(0) as u64;
    let tenths = int_value * 10 + digit(0) + if digit(1) >= 5 { 1 } else { 0 };

    Some(if tenths % 10 == 0 {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    })
}

fn people(list: Option<Node>, kind: &str, max: usize) -> Vec<Person> {
    let mut out: Vec<Person> = Vec::new();
    let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
    let Some(list) = list else { return out };

    for group in list
        .children()
        .filter(|o| is_el(o, "o") && o.attribute("t") == Some(kind))
    {
        for j in group.children().filter(|j| is_el(j, "j")) {
            let name = html_decode(&value(j)).trim().to_string();
            if name.is_empty() {
                continue;
            }
            let role = blank(&html_decode(j.attribute("role").unwrap_or("")));
            if !seen.insert((name.clone(), role.clone())) {
                continue;
            }
            out.push(Person { name, role });
            if out.len() >= max {
                return out;
            }
        }
    }
    out
}

fn read_detail(a: Node) -> DetailInfo {
    let info = child(a, "i");
    let list = info.and_then(|i| child(i, "l"));

    // <o t="..."> groups, confirmed against real files: r = director, s = writer, p = producer,
    // m = composer, h = cast (with a role attribute). k (cinematographer) has no XMLTV equivalent.
    let mut tags: Vec<String> = Vec::new();
    let mut seen_tags: HashSet<String> = HashSet::new();
    if let Some(st) = info.and_then(|i| child(i, "st")) {
        for tt in st.children().filter(|t| is_el(t, "tt")) {
            let tag = value(tt).trim().to_string();
            if !tag.is_empty() && seen_tags.insert(tag.to_lowercase()) {
                tags.push(tag);
            }
        }
    }

    let mut icon: Option<String> = child(a, "g").and_then(|g| {
        g.children()
            .filter(|o| is_el(o, "o"))
            .map(|o| value(o).trim().to_string())
            .find(|u| !u.is_empty())
    });
    if icon.is_none() {
        icon = opt_blank(child_value(a, "o"));
    }
    // "/zanr/" images are generic genre placeholders, not pictures of the programme.
    let bad_icon = icon
        .as_deref()
        .is_some_and(|i| i.contains("/zanr/") || !i.starts_with("http"));
    if bad_icon {
        icon = None;
    }

    let rating = opt_blank(child_value(a, "h")).and_then(|r| parse_rating(&r.replace(',', ".")));

    let flags: HashSet<String> = info
        .and_then(|i| child(i, "qq"))
        .map(|qq| {
            qq.children()
                .filter(|q| is_el(q, "q"))
                .filter_map(|q| q.attribute("id"))
                .filter(|id| !id.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let info_text = |name: &str| info.and_then(|i| child_value(i, name));

    DetailInfo {
        original_title: opt_blank(child_value(a, "b").map(|b| html_decode(&b))),
        kind: opt_blank(info_text("t")),
        tags,
        directors: people(list, "r", 4),
        writers: people(list, "s", 6),
        producers: people(list, "p", 6),
        composers: people(list, "m", 4),
        actors: people(list, "h", 25),
        country: opt_blank(info_text("z")),
        year: parse_int(info_text("r").as_deref(), 1850, Utc::now().year() as i64 + 2).map(|n| n as i32),
        minutes: parse_int(info_text("d").as_deref(), 1, 1500).map(|n| n as u32),
        rating,
        icon,
        flags,
    }
}

// =====================================================================
//  Title: "Name", "Name (3)", "Name (3/12)", "Name II (13)"
// =====================================================================

pub fn parse_title(title: &str, clean_title: Option<&str>) -> (String, Option<EpisodeInfo>) {
    if let Some(clean) = clean_title.filter(|c| !c.trim().is_empty()) {
        if let Some(rest) = title.strip_prefix(clean) {
            if rest.trim().is_empty() {
                return (clean.to_string(), None);
            }
            if let Some(m) = SUFFIX_RX.captures(rest) {
                // The marker is dropped even if it is not usable.
                return (clean.to_string(), build_episode(&m).0);
            }
        }
    }

    let Some(m) = TITLE_EPISODE_RX.captures(title) else {
        return (title.to_string(), None);
    };

    let mut name = m["name"].trim_end().to_string();
    if name.is_empty() {
        return (title.to_string(), None);
    }

    let (episode, roman_is_not_a_numeral) = build_episode(&m);
    let Some(episode) = episode else {
        return (title.to_string(), None);
    };

    if roman_is_not_a_numeral {
        // e.g. a word that merely looks Roman
        name = format!("{} {}", name, &m["roman"]);
    }
    (name, Some(episode))
}

/// The episode, and whether the "roman numeral" in the title turned out not to be one.
fn build_episode(m: &Captures) -> (Option<EpisodeInfo>, bool) {
    let episode: u32 = m["ep"].parse().unwrap_or(0);
    let mut total: Option<u32> = m.name("tot").and_then(|t| t.as_str().parse().ok());

    if episode == 0 {
        return (None, false);
    }
    if total.is_some_and(|t| episode > t) {
        total = None; // "Inside Africa XXVI (737/26)"
    }

    let mut season = None;
    let mut roman_is_not_a_numeral = false;
    if let Some(roman) = m.name("roman") {
        season = roman_to_int(roman.as_str());
        roman_is_not_a_numeral = season.is_none();
    }

    (Some(EpisodeInfo { season, episode, total }), roman_is_not_a_numeral)
}

pub fn roman_to_int(s: &str) -> Option<u32> {
    if s.is_empty() || !ROMAN_RX.is_match(s) {
        return None;
    }

    let (mut total, mut prev) = (0i32, 0i32);
    for c in s.chars().rev() {
        let v = match c {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            _ => 0,
        };
        total += if v < prev { -v } else { v };
        prev = v;
    }
    u32::try_from(total).ok()
}

/// xmltv_ns is zero-based "season.episode/total.part", e.g. S02E13 -> "1.12.".
pub fn to_xmltv_ns(e: &EpisodeInfo) -> String {
    let season = e.season.map(|s| s.saturating_sub(1).to_string()).unwrap_or_default();
    let mut episode = e.episode.saturating_sub(1).to_string();
    if let Some(total) = e.total {
        episode.push('/');
        episode.push_str(&total.to_string());
    }
    format!("{season}.{episode}.")
}

pub fn to_on_screen(e: &EpisodeInfo) -> String {
    match e.season {
        Some(s) => format!("S{:02}E{:02}", s, e.episode),
        None => format!("E{:02}", e.episode),
    }
}

// =====================================================================
//  Extract
// =====================================================================

fn add_category(categories: &mut Vec<Category>, lang: &'static str, text: String) {
    if !categories.iter().any(|c| eq_ignore_case(&c.text, &text)) {
        categories.push(Category { lang, text });
    }
}

pub fn extract(
    raw_title: &str,
    list_title: Option<&str>,
    detail: Option<&DetailData>,
    fallback_short: Option<&str>,
) -> ProgrammeInfo {
    let d: Option<&DetailInfo> = detail.map(|x| &x.info);

    let title = SPACES_RX
        .replace_all(&raw_title.replace('\u{00A0}', " "), " ")
        .trim()
        .to_string();
    let (clean_title, episode) = parse_title(&title, list_title.map(str::trim));

    let short_source: Option<&str> = match detail.and_then(|x| x.short_desc.as_deref()) {
        Some(s) if !s.trim().is_empty() => Some(s),
        _ => fallback_short,
    };

    let s0 = prepare(short_source);
    let l0 = prepare(detail.and_then(|x| x.long_desc.as_deref()));

    // For series <b> is the episode's original title ("Problem at Sea"). Skip it when it only repeats
    // the series name ("Zoom In") or is a placeholder ("Episode #8.5").
    let mut sub_title: Option<String> = None;
    if episode.is_some() {
        if let Some(original) = d.and_then(|x| x.original_title.as_deref()) {
            if !eq_ignore_case(original, &clean_title) && !EPISODE_PLACEHOLDER_RX.is_match(original) {
                sub_title = Some(original.to_string());
            }
        }
    }

    // The original title is written in front of the description: "(The Garfield Show I) Kocour Garfield ...".
    // Take it out of the text; it becomes a second <title> (see `clean_original_title`).
    let (s, prefix_from_short) = strip_original_title(s0);
    let (l, prefix_from_long) = strip_original_title(l0);
    let original_title = clean_original_title(
        prefix_from_short.or(prefix_from_long),
        episode.as_ref(),
        &clean_title,
        sub_title.as_deref(),
    );

    let all = [s.as_deref(), l.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");

    // --- credits: structured first, text only when the structured list is empty ---
    let directors = match d {
        Some(x) if !x.directors.is_empty() => x.directors.clone(),
        _ => to_people(find_names(&DIRECTOR_RX, &all, 4)),
    };
    let actors = match d {
        Some(x) if !x.actors.is_empty() => x.actors.clone(),
        _ => to_people(find_names(&ACTOR_RX, &all, 20)),
    };
    // Presenters are not part of the structured list, so they come from the text (short description first).
    let mut presenter_names = find_names(&PRESENTER_RX, s.as_deref().unwrap_or(""), 4);
    if presenter_names.is_empty() {
        if let Some(l) = &l {
            presenter_names = find_names(&PRESENTER_RX, l, 4);
        }
    }
    let presenters = to_people(presenter_names);

    let year = d
        .and_then(|x| x.year)
        .or_else(|| year_from_text(s.as_deref(), l.as_deref()));

    // --- categories: the server's own type + sub-genre tags ---
    let mut categories: Vec<Category> = Vec::new();
    if let Some(kind) = d.and_then(|x| x.kind.as_deref()) {
        match type_map(kind) {
            Some(english) => add_category(&mut categories, "en", english.to_string()),
            None => add_category(&mut categories, "cs", kind.to_string()),
        }
    } else if episode.is_some() {
        add_category(&mut categories, "en", "Series".to_string());
    }
    if let Some(x) = d {
        for tag in &x.tags {
            add_category(&mut categories, "cs", tag.clone());
        }
    }

    // Flag codes from <qq><q id="...">, as listed by the server:
    // r repríza, p premiéra, s stereo, f HD, w širokoúhlé (16:9), h skryté titulky (teletext),
    // t titulky (on screen), z znakový jazyk (sign language), c černobílý film, l živé vysílání, d duální vysílání.
    let has = |id: &str| d.is_some_and(|x| x.flags.contains(id));

    let mut subtitle_types: Vec<&'static str> = Vec::new();
    if has("h") {
        subtitle_types.push("teletext");
    }
    if has("t") {
        subtitle_types.push("onscreen");
    }
    if has("z") {
        subtitle_types.push("deaf-signed");
    }

    let premiere = has("p")
        || s.as_deref().is_some_and(|t| PREMIERE_RX.is_match(t))
        || l.as_deref().is_some_and(|t| PREMIERE_RX.is_match(t));
    let rerun = has("r") || RERUN_RX.is_match(&all);

    ProgrammeInfo {
        title: clean_title,
        raw_title: title,
        sub_title,
        original_title,
        description: merge_descriptions(s.as_deref(), l.as_deref()),
        episode,
        year,
        country: d.and_then(|x| x.country.clone()),
        length_minutes: d.and_then(|x| x.minutes),
        directors,
        actors,
        writers: d.map(|x| x.writers.clone()).unwrap_or_default(),
        producers: d.map(|x| x.producers.clone()).unwrap_or_default(),
        composers: d.map(|x| x.composers.clone()).unwrap_or_default(),
        presenters,
        categories,
        rating: d.and_then(|x| x.rating.as_ref()).map(|r| format!("{r}/10")),
        icon: d.and_then(|x| x.icon.clone()),
        quality: if has("f") { Some("HDTV") } else { None },
        aspect: if has("w") { Some("16:9") } else { None },
        black_and_white: has("c"),
        audio: if has("s") { Some("stereo") } else { None },
        subtitle_types,
        premiere,
        previously_shown: rerun,
    }
}

// =====================================================================
//  Original title ("(The Garfield Show I) ..." at the start of the description)
// =====================================================================

/// Removes a leading "(Original title)" from the text and returns it separately: (text, original title).
fn strip_original_title(text: Option<String>) -> (Option<String>, Option<String>) {
    let Some(text) = text else {
        return (None, None);
    };

    let parts = ORIGINAL_TITLE_PREFIX_RX
        .captures(&text)
        .map(|m| (m["t"].to_string(), m["rest"].to_string()));
    let Some((inner_raw, rest_raw)) = parts else {
        return (Some(text), None);
    };

    let inner = SPACES_RX.replace_all(&inner_raw, " ").trim().to_string();
    if !looks_like_title(&inner) {
        // "(2024)", "(R)", "(AD)", "(vom 20.10.2022)" stay as they are
        return (Some(text), None);
    }

    let rest = rest_raw.trim().to_string();
    if count_letters(&rest) < 5 {
        (None, Some(inner))
    } else {
        (Some(rest), Some(inner))
    }
}

fn looks_like_title(t: &str) -> bool {
    if count_letters(t) < 3 {
        return false; // "2024", "6", "R"
    }
    if t.chars().count() <= 4 && t.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        return false; // "AD", "HD"
    }
    if DATE_LIKE_RX.is_match(t) || COUNTRY_YEAR_RX.is_match(t) || FLAG_WORD_RX.is_match(t) {
        return false;
    }
    t.split(' ').filter(|w| !w.is_empty()).count() <= 12 // longer is a sentence
}

/// Decides whether the stripped title is worth adding as a second title: the season numeral that
/// repeats the Czech title's season is removed ("Cold Case V"), and titles that are the same as,
/// or only a slight variation of, the main title or the sub-title are dropped.
fn clean_original_title(
    raw: Option<String>,
    episode: Option<&EpisodeInfo>,
    clean_title: &str,
    sub_title: Option<&str>,
) -> Option<String> {
    let raw = raw?;

    let mut t = raw.as_str();
    if let Some(m) = TRAILING_ROMAN_RX.captures(t) {
        let season = episode.and_then(|e| e.season);
        let numeral = roman_to_int(&m["r"]);
        if season.is_some() && numeral == season {
            let cut = m.get(0).map_or(t.len(), |whole| whole.start());
            t = t[..cut].trim_end();
        }
    }

    let k = key(t);
    if t.chars().count() < 3 || k.chars().count() < 3 {
        return None;
    }

    for other in [Some(clean_title), sub_title].into_iter().flatten() {
        if similar(&k, &key(other)) {
            return None;
        }
    }
    Some(t.to_string())
}

fn similar(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (shorter, longer) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    let (ls, ll) = (shorter.chars().count(), longer.chars().count());
    ls >= 3 && longer.contains(shorter) && ls * 2 > ll
}

// =====================================================================
//  Text fallbacks (only used when the structured data has nothing)
// =====================================================================

fn year_from_text(s: Option<&str>, l: Option<&str>) -> Option<i32> {
    for text in [s, l].into_iter().flatten() {
        let Some(m) = YEAR_RX.captures(text) else { continue };
        let year: i32 = m["year"].parse().ok()?;
        if year <= Utc::now().year() + 1 {
            return Some(year);
        }
    }
    None
}

fn is_noise(word: &str) -> bool {
    const NOISE: &[&str] = &[
        "moderuje", "moderují", "režie", "réžia", "hrají", "účinkují", "účinkuje", "účinkujú", "regie",
        "moderation", "rendező", "szereplők", "hudba", "scénář", "kamera", "die", "der", "das", "ein",
        "eine", "hier", "themen", "heute", "und", "im", "in", "am", "zu", "mit", "wir", "sledujte", "více",
    ];
    NOISE.contains(&word.to_lowercase().as_str())
}

fn clean_name(raw: &str) -> Option<String> {
    let mut words: Vec<&str> = raw.split(' ').map(str::trim).filter(|w| !w.is_empty()).collect();
    while words.last().is_some_and(|w| is_noise(w)) {
        words.pop();
    }
    while words.first().is_some_and(|w| is_noise(w)) {
        words.remove(0);
    }
    if words.is_empty() {
        return None;
    }

    let name = words.join(" ");
    let len = name.chars().count();
    if (3..=40).contains(&len) {
        Some(name)
    } else {
        None
    }
}

/// The name list of the first credit label ("Režie: ...", "Hrají ...") that does not start in the
/// middle of a word.
fn find_credit<'t>(rx: &Regex, text: &'t str) -> Option<&'t str> {
    let mut pos = 0;
    while pos <= text.len() {
        let caps = rx.captures_at(text, pos)?;
        let whole = caps.get(0)?;

        let after_letter = text[..whole.start()]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphabetic());
        if !after_letter {
            return caps.name("list").map(|list| list.as_str());
        }

        // Preceded by a letter: not a label. Try again from the next character.
        let step = text[whole.start()..].chars().next().map_or(1, char::len_utf8);
        pos = whole.start() + step;
    }
    None
}

fn find_names(rx: &Regex, text: &str, max: usize) -> Vec<String> {
    let Some(list) = find_credit(rx, text) else {
        return Vec::new();
    };

    let mut seen: HashSet<String> = HashSet::new();
    let mut names = Vec::new();
    for m in NAME_RX.find_iter(list) {
        let Some(name) = clean_name(m.as_str()) else { continue };
        if seen.insert(name.to_lowercase()) {
            names.push(name);
            if names.len() >= max {
                break;
            }
        }
    }
    names
}

fn to_people(names: Vec<String>) -> Vec<Person> {
    names.into_iter().map(|name| Person { name, role: None }).collect()
}

// =====================================================================
//  Descriptions
// =====================================================================

fn prepare(text: Option<&str>) -> Option<String> {
    let text = text?;
    if text.trim().is_empty() {
        return None;
    }

    let stripped = HTML_TAG_RX.replace_all(text, " ");
    let decoded = html_decode(&stripped).replace('\u{00A0}', " ");

    let mut sb = String::with_capacity(decoded.len());
    for line in decoded
        .split(|c: char| c == '\r' || c == '\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        if let Some(last) = sb.chars().next_back() {
            sb.push_str(if ".!?…:;,".contains(last) { " " } else { ". " });
        }
        sb.push_str(line);
    }

    let collapsed = SPACES_RX.replace_all(&sb, " ");
    let result = LEADING_PERCENT_RX.replace(collapsed.trim(), "").into_owned();

    // After removing the percentage some descriptions are only "(2020)" or "2026." - not worth keeping.
    if count_letters(&result) < 5 {
        None
    } else {
        Some(result)
    }
}

fn starts_sentence(c: char) -> bool {
    c.is_uppercase() || c.is_numeric() || matches!(c, '„' | '"' | '\'' | '‚' | '(')
}

/// Splits after sentence-ending punctuation when the next sentence starts with a capital, a quote,
/// a bracket or a digit.
fn split_sentences(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < chars.len() {
        if !chars[i].1.is_whitespace() {
            i += 1;
            continue;
        }

        let run_start = i;
        while i < chars.len() && chars[i].1.is_whitespace() {
            i += 1;
        }

        let after_stop = run_start > 0 && matches!(chars[run_start - 1].1, '.' | '!' | '?' | '…');
        let before_start = i < chars.len() && starts_sentence(chars[i].1);
        if after_stop && before_start {
            parts.push(&text[start..chars[run_start].0]);
            start = chars[i].0;
        }
    }

    parts.push(&text[start..]);
    parts
}

fn covers(big: &str, small: &str) -> bool {
    if small.chars().count() >= 12 {
        big.contains(small)
    } else {
        big == small
    }
}

/// The API sends a short and a long description that mostly repeat each other.
/// Merge them sentence by sentence, dropping sentences that are already covered.
fn merge_descriptions(short_text: Option<&str>, long_text: Option<&str>) -> Option<String> {
    // Only one of them: nothing to merge.
    let (short_text, long_text) = match (short_text, long_text) {
        (Some(s), Some(l)) => (s, l),
        (s, l) => return s.or(l).map(String::from),
    };

    let mut items: Vec<(String, String)> = Vec::new(); // (sentence, key)

    for text in [short_text, long_text] {
        for sentence in split_sentences(text) {
            let k = key(sentence);
            if k.is_empty() {
                continue;
            }

            // already said (possibly longer)
            if items.iter().any(|(_, existing)| covers(existing, &k)) {
                continue;
            }

            // this one extends an earlier one
            match items.iter().position(|(_, existing)| covers(&k, existing)) {
                Some(idx) => items[idx] = (sentence.to_string(), k),
                None => items.push((sentence.to_string(), k)),
            }
        }
    }

    if items.is_empty() {
        return None;
    }

    // A sentence from the API often has no final full stop; add one before the next sentence
    // is appended so they don't run together.
    let mut texts: Vec<String> = items.into_iter().map(|(t, _)| t).collect();
    let last = texts.len() - 1;
    for text in &mut texts[..last] {
        if let Some(end) = text.chars().next_back() {
            if !".!?…\"”)»’:;".contains(end) {
                text.push('.');
            }
        }
    }

    Some(texts.join(" "))
}

// =====================================================================
//  Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(season: Option<u32>, episode: u32, total: Option<u32>) -> EpisodeInfo {
        EpisodeInfo { season, episode, total }
    }

    #[test]
    fn title_with_clean_name() {
        let (title, ep) = parse_title("Kocour Garfield (3)", Some("Kocour Garfield"));
        assert_eq!(title, "Kocour Garfield");
        assert_eq!(ep, Some(episode(None, 3, None)));
    }

    #[test]
    fn title_with_season_and_total() {
        let (title, ep) = parse_title("Cold Case V (13/22)", None);
        assert_eq!(title, "Cold Case");
        assert_eq!(ep, Some(episode(Some(5), 13, Some(22))));
    }

    #[test]
    fn total_smaller_than_episode_is_dropped() {
        let (title, ep) = parse_title("Inside Africa XXVI (737/26)", None);
        assert_eq!(title, "Inside Africa");
        assert_eq!(ep, Some(episode(Some(26), 737, None)));
    }

    #[test]
    fn title_without_marker_is_unchanged() {
        let (title, ep) = parse_title("Zprávy", None);
        assert_eq!(title, "Zprávy");
        assert_eq!(ep, None);
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman_to_int("IV"), Some(4));
        assert_eq!(roman_to_int("XXVI"), Some(26));
        assert_eq!(roman_to_int("IIII"), None);
        assert_eq!(roman_to_int("XXXX"), None);
        assert_eq!(roman_to_int(""), None);
    }

    #[test]
    fn episode_numbers_are_formatted() {
        assert_eq!(to_xmltv_ns(&episode(Some(2), 13, Some(26))), "1.12/26.");
        assert_eq!(to_xmltv_ns(&episode(None, 5, None)), ".4.");
        assert_eq!(to_on_screen(&episode(Some(2), 13, None)), "S02E13");
        assert_eq!(to_on_screen(&episode(None, 5, None)), "E05");
    }

    #[test]
    fn rating_is_rounded_like_dotnet() {
        assert_eq!(parse_rating("6.3").as_deref(), Some("6.3"));
        assert_eq!(parse_rating("7.0").as_deref(), Some("7"));
        assert_eq!(parse_rating("6.25").as_deref(), Some("6.3"));
        assert_eq!(parse_rating("9.96").as_deref(), Some("10"));
        assert_eq!(parse_rating("10").as_deref(), Some("10"));
        assert_eq!(parse_rating("11"), None);
        assert_eq!(parse_rating("10.04"), None);
        assert_eq!(parse_rating("abc"), None);
        assert_eq!(parse_rating("1e3"), None);
    }

    #[test]
    fn descriptions_are_merged_sentence_by_sentence() {
        let merged = merge_descriptions(
            Some("Foo bar baz qux. Second sentence here."),
            Some("Foo bar baz qux. Second sentence here. Third one follows."),
        );
        assert_eq!(merged.as_deref(), Some("Foo bar baz qux. Second sentence here. Third one follows."));
        assert_eq!(merge_descriptions(None, Some("Jen dlouhý popis.")).as_deref(), Some("Jen dlouhý popis."));
        assert_eq!(merge_descriptions(None, None), None);
    }

    #[test]
    fn sentences_split_only_before_a_sentence_start() {
        assert_eq!(split_sentences("Ahoj světe. Jak se máš? dobře"), vec!["Ahoj světe.", "Jak se máš? dobře"]);
        assert_eq!(split_sentences("Bez tečky"), vec!["Bez tečky"]);
    }

    #[test]
    fn prepare_cleans_markup_and_rating_prefix() {
        let text = prepare(Some("86% <b>Britský</b> seriál&nbsp;podle\nknihy"));
        assert_eq!(text.as_deref(), Some("Britský seriál podle. knihy"));
        assert_eq!(prepare(Some("(2020)")), None);
        assert_eq!(prepare(Some("   ")), None);
    }

    #[test]
    fn original_title_prefix_is_stripped() {
        let (text, original) = strip_original_title(Some("(The Garfield Show) Kocour Garfield a jeho přátelé.".into()));
        assert_eq!(original.as_deref(), Some("The Garfield Show"));
        assert_eq!(text.as_deref(), Some("Kocour Garfield a jeho přátelé."));

        let (text, original) = strip_original_title(Some("(2024) Film o něčem důležitém.".into()));
        assert_eq!(original, None);
        assert_eq!(text.as_deref(), Some("(2024) Film o něčem důležitém."));
    }

    #[test]
    fn credits_are_found_in_text() {
        assert_eq!(
            find_names(&DIRECTOR_RX, "Režie: Jan Svěrák. Hrají: Zdeněk Svěrák", 4),
            vec!["Jan Svěrák"]
        );
        assert_eq!(
            find_names(&ACTOR_RX, "Hrají: Jiří Bartoška, Jan Tříska a Eva Holubová", 20),
            vec!["Jiří Bartoška", "Jan Tříska", "Eva Holubová"]
        );
        assert!(find_names(&ACTOR_RX, "Nic o hercích.", 20).is_empty());
    }

    #[test]
    fn extract_without_detail_uses_listing_data() {
        let info = extract("Kocour Garfield (3)", Some("Kocour Garfield"), None, Some("Krátký popis seriálu."));
        assert_eq!(info.title, "Kocour Garfield");
        assert_eq!(info.raw_title, "Kocour Garfield (3)");
        assert_eq!(info.episode, Some(episode(None, 3, None)));
        assert_eq!(info.description.as_deref(), Some("Krátký popis seriálu."));
        assert_eq!(info.categories.len(), 1);
        assert_eq!(info.categories[0].text, "Series");
    }
}
