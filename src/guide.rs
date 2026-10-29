//! Reads an XMLTV file (plain or gzip) into memory so the terminal UI can browse it.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate};
use flate2::read::GzDecoder;
use roxmltree::Node;

use crate::groups::normalize;
use crate::util::{child_value, is_el, value};

pub struct Guide {
    pub channels: Vec<GuideChannel>,
    /// Every local date on which something starts, ascending.
    pub dates: Vec<NaiveDate>,
    pub programme_count: usize,
}

pub struct GuideChannel {
    pub id: String,
    pub name: String,
    /// Lower-case, diacritics removed: name and id, for the channel filter.
    pub search: String,
    /// Sorted by start time.
    pub programmes: Vec<GuideProgramme>,
}

pub struct GuideProgramme {
    pub start: DateTime<FixedOffset>,
    pub stop: Option<DateTime<FixedOffset>>,
    pub title: String,
    pub sub_title: Option<String>,
    pub description: Option<String>,
    pub categories: Vec<String>,
    /// (kind, name): ("actor", "Jan Novák (Karel)")
    pub credits: Vec<(String, String)>,
    pub year: Option<String>,
    pub length: Option<String>,
    pub country: Option<String>,
    /// The on-screen episode number, e.g. "S2 E5".
    pub episode: Option<String>,
    pub rating: Option<String>,
    pub quality: Option<String>,
    pub premiere: bool,
    pub rerun: bool,
    /// Lower-case, diacritics removed: title and sub-title, for the search.
    pub search: String,
}

impl GuideProgramme {
    /// Short flags for the programme list: NEW, RERUN, HDTV...
    pub fn tags(&self) -> Vec<String> {
        let mut tags = Vec::new();
        if self.premiere {
            tags.push("NEW".to_string());
        }
        if self.rerun {
            tags.push("RERUN".to_string());
        }
        if let Some(quality) = &self.quality {
            tags.push(quality.clone());
        }
        tags
    }
}

impl GuideChannel {
    /// Positions (in `programmes`) of the programmes that start on this local date.
    pub fn on(&self, day: NaiveDate) -> Vec<usize> {
        self.programmes
            .iter()
            .enumerate()
            .filter(|(_, p)| p.start.date_naive() == day)
            .map(|(i, _)| i)
            .collect()
    }

    /// When the programme at `index` ends: its own stop time, or else the start of the next one.
    pub fn end(&self, index: usize) -> Option<DateTime<FixedOffset>> {
        self.programmes[index].stop.or_else(|| self.programmes.get(index + 1).map(|next| next.start))
    }
}

impl Guide {
    /// (channel, programme) positions of programmes whose title matches, earliest first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, usize)> {
        let q = normalize(query);
        if q.is_empty() {
            return Vec::new();
        }

        let mut hits: Vec<(usize, usize)> = Vec::new();
        for (c, channel) in self.channels.iter().enumerate() {
            for (p, programme) in channel.programmes.iter().enumerate() {
                if programme.search.contains(&q) {
                    hits.push((c, p));
                }
            }
        }
        hits.sort_by_key(|&(c, p)| self.channels[c].programmes[p].start);
        hits.truncate(limit);
        hits
    }
}

pub fn load(path: &Path) -> Result<Guide, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

    let bytes = if bytes.starts_with(&[0x1fu8, 0x8b]) {
        let mut plain = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut plain)
            .map_err(|e| format!("Cannot decompress {}: {e}", path.display()))?;
        plain
    } else {
        bytes
    };

    let text = String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8.", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_time(text: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_str(text.trim(), "%Y%m%d%H%M%S %z").ok()
}

fn non_blank(text: String) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn parse(text: &str) -> Result<Guide, String> {
    // Files from other generators often start with a <!DOCTYPE tv SYSTEM "xmltv.dtd"> line.
    let mut options = roxmltree::ParsingOptions::default();
    options.allow_dtd = true;
    let doc = roxmltree::Document::parse_with_options(text, options).map_err(|e| format!("Not valid XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "tv" {
        return Err("Not an XMLTV file (the root element is not <tv>).".to_string());
    }

    let mut channels: Vec<GuideChannel> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for node in root.children().filter(|n| is_el(n, "channel")) {
        let id = node.attribute("id").unwrap_or("").to_string();
        let name = child_value(node, "display-name").and_then(non_blank).unwrap_or_else(|| id.clone());
        let search = normalize(&format!("{name} {id}"));
        index.insert(id.clone(), channels.len());
        channels.push(GuideChannel { id, name, search, programmes: Vec::new() });
    }

    let mut programme_count = 0usize;
    let mut dates: BTreeSet<NaiveDate> = BTreeSet::new();
    for node in root.children().filter(|n| is_el(n, "programme")) {
        let Some(&channel) = node.attribute("channel").and_then(|c| index.get(c)) else { continue };
        let Some(start) = node.attribute("start").and_then(parse_time) else { continue };

        let programme = programme_from(node, start);
        dates.insert(programme.start.date_naive());
        channels[channel].programmes.push(programme);
        programme_count += 1;
    }

    for channel in &mut channels {
        channel.programmes.sort_by_key(|p| p.start);
    }

    Ok(Guide { channels, dates: dates.into_iter().collect(), programme_count })
}

fn programme_from(node: Node, start: DateTime<FixedOffset>) -> GuideProgramme {
    let title = child_value(node, "title").and_then(non_blank).unwrap_or_default();
    let sub_title = child_value(node, "sub-title").and_then(non_blank);
    let search = normalize(&format!("{title} {}", sub_title.as_deref().unwrap_or("")));

    let mut p = GuideProgramme {
        start,
        stop: node.attribute("stop").and_then(parse_time),
        title,
        sub_title,
        description: child_value(node, "desc").and_then(non_blank),
        categories: Vec::new(),
        credits: Vec::new(),
        year: child_value(node, "date").and_then(non_blank),
        length: child_value(node, "length").and_then(non_blank),
        country: child_value(node, "country").and_then(non_blank),
        episode: None,
        rating: None,
        quality: None,
        premiere: false,
        rerun: false,
        search,
    };

    for c in node.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "category" => {
                if let Some(category) = non_blank(value(c)) {
                    p.categories.push(category);
                }
            }
            "credits" => {
                for person in c.children().filter(|n| n.is_element()) {
                    let Some(name) = non_blank(value(person)) else { continue };
                    let text = match person.attribute("role").map(str::trim).filter(|r| !r.is_empty()) {
                        Some(role) => format!("{name} ({role})"),
                        None => name,
                    };
                    p.credits.push((person.tag_name().name().to_string(), text));
                }
            }
            "episode-num" => {
                if c.attribute("system") == Some("onscreen") {
                    p.episode = non_blank(value(c));
                }
            }
            "video" => p.quality = child_value(c, "quality").and_then(non_blank),
            "star-rating" => p.rating = child_value(c, "value").and_then(non_blank),
            "premiere" => p.premiere = true,
            "previously-shown" => p.rerun = true,
            _ => {}
        }
    }

    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE tv SYSTEM "xmltv.dtd">
<tv>
  <channel id="cz-nova-2"><display-name lang="cs">Nova</display-name></channel>
  <channel id="sk-markiza-3"><display-name lang="cs">Markíza</display-name></channel>
  <programme start="20261028200000 +0100" stop="20261028213000 +0100" channel="cz-nova-2">
    <title>Večer s hvězdami</title>
    <sub-title>Díl 3</sub-title>
    <desc>Popis pořadu.</desc>
    <credits><director>Jan Novák</director><actor role="Karel">Petr Svoboda</actor></credits>
    <date>2025</date>
    <category lang="cs">Zábava</category>
    <episode-num system="onscreen">S1 E3</episode-num>
    <video><quality>HDTV</quality></video>
    <premiere />
  </programme>
  <programme start="20261028180000 +0100" channel="cz-nova-2"><title>Zprávy</title></programme>
  <programme start="20261029060000 +0100" channel="sk-markiza-3"><title>Ranní show</title></programme>
  <programme start="nonsense" channel="sk-markiza-3"><title>Bad</title></programme>
  <programme start="20261029060000 +0100" channel="unknown"><title>Orphan</title></programme>
</tv>"#;

    #[test]
    fn channels_and_programmes_are_read() {
        let g = parse(SAMPLE).unwrap();
        assert_eq!(g.channels.len(), 2);
        assert_eq!(g.programme_count, 3);
        assert_eq!(g.channels[0].name, "Nova");

        // sorted by start, although the file lists the evening show first
        let titles: Vec<&str> = g.channels[0].programmes.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["Zprávy", "Večer s hvězdami"]);
    }

    #[test]
    fn details_are_read() {
        let g = parse(SAMPLE).unwrap();
        let p = &g.channels[0].programmes[1];
        assert_eq!(p.sub_title.as_deref(), Some("Díl 3"));
        assert_eq!(p.description.as_deref(), Some("Popis pořadu."));
        assert_eq!(p.year.as_deref(), Some("2025"));
        assert_eq!(p.episode.as_deref(), Some("S1 E3"));
        assert_eq!(p.categories, vec!["Zábava".to_string()]);
        assert_eq!(
            p.credits,
            vec![
                ("director".to_string(), "Jan Novák".to_string()),
                ("actor".to_string(), "Petr Svoboda (Karel)".to_string())
            ]
        );
        assert_eq!(p.tags(), vec!["NEW".to_string(), "HDTV".to_string()]);
    }

    #[test]
    fn days_and_end_times() {
        let g = parse(SAMPLE).unwrap();
        assert_eq!(g.dates.len(), 2);
        let day = g.dates[0];
        assert_eq!(g.channels[0].on(day), vec![0, 1]);
        assert_eq!(g.channels[1].on(day), Vec::<usize>::new());

        // "Zprávy" has no stop time: it ends when the next programme starts
        assert_eq!(g.channels[0].end(0), Some(g.channels[0].programmes[1].start));
        // the last one has neither
        assert_eq!(g.channels[1].end(0), None);
    }

    #[test]
    fn search_ignores_case_and_diacritics() {
        let g = parse(SAMPLE).unwrap();
        let hits = g.search("VECER", 10);
        assert_eq!(hits, vec![(0, 1)]);
        assert_eq!(g.search("", 10), Vec::<(usize, usize)>::new());
        assert_eq!(g.search("rann", 10), vec![(1, 0)]);
    }

    #[test]
    fn a_non_xmltv_file_is_rejected() {
        assert!(parse("<html/>").is_err());
        assert!(parse("not xml").is_err());
    }
}
