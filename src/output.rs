//! XMLTV output. The XML is written by hand (2-space indentation, same layout as the C# version):
//! the document is simple, and it lets each programme be serialised straight to a temporary file.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::enrichment::{to_on_screen, to_xmltv_ns, Person};
use crate::{Channel, Options, Programme};

const LOGO_BASE: &str = "https://www.sms.cz/kategorie/televize/bmp/loga/velka/";

// ---------- low-level writing ----------

/// Characters allowed in XML 1.0. Anything else would make the file invalid, so it is dropped.
fn is_xml_char(c: char) -> bool {
    matches!(c,
        '\t' | '\n' | '\r'
        | '\u{20}'..='\u{D7FF}'
        | '\u{E000}'..='\u{FFFD}'
        | '\u{10000}'..='\u{10FFFF}')
}

fn push_escaped(out: &mut String, text: &str, attribute: bool) {
    for c in text.chars().filter(|&c| is_xml_char(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\n' if attribute => out.push_str("&#xA;"),
            '\t' if attribute => out.push_str("&#x9;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn tag_start(out: &mut String, depth: usize, name: &str, attrs: &[(&str, &str)]) {
    indent(out, depth);
    out.push('<');
    out.push_str(name);
    for (key, value) in attrs {
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        push_escaped(out, value, true);
        out.push('"');
    }
}

/// `<name attr="..">` followed by children on their own lines.
fn open(out: &mut String, depth: usize, name: &str, attrs: &[(&str, &str)]) {
    tag_start(out, depth, name, attrs);
    out.push_str(">\n");
}

fn close(out: &mut String, depth: usize, name: &str) {
    indent(out, depth);
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

/// `<name attr="..">text</name>`
fn leaf(out: &mut String, depth: usize, name: &str, attrs: &[(&str, &str)], text: &str) {
    tag_start(out, depth, name, attrs);
    out.push('>');
    push_escaped(out, text, false);
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

/// `<name attr=".." />`
fn empty(out: &mut String, depth: usize, name: &str, attrs: &[(&str, &str)]) {
    tag_start(out, depth, name, attrs);
    out.push_str(" />\n");
}

// ---------- elements ----------

pub fn write_channel(out: &mut String, ch: &Channel) {
    open(out, 1, "channel", &[("id", ch.xml_id.as_str())]);
    leaf(out, 2, "display-name", &[("lang", "cs")], &ch.name);
    if let Some(logo) = ch.logo.as_deref().filter(|l| !l.trim().is_empty()) {
        let src = format!("{LOGO_BASE}{logo}");
        empty(out, 2, "icon", &[("src", src.as_str())]);
    }
    close(out, 1, "channel");
}

fn person(out: &mut String, depth: usize, tag: &str, p: &Person) {
    match p.role.as_deref() {
        Some(role) => leaf(out, depth, tag, &[("role", role)], &p.name),
        None => leaf(out, depth, tag, &[], &p.name),
    }
}

/// Children are added in the order the XMLTV DTD requires.
pub fn write_programme(out: &mut String, p: &Programme, channel_id: &str, opts: &Options) {
    let i = &p.info;

    let mut attrs: Vec<(&str, &str)> = vec![("start", p.start.as_str()), ("channel", channel_id)];
    if let Some(stop) = p.stop.as_deref() {
        attrs.push(("stop", stop));
    }
    open(out, 1, "programme", &attrs);

    leaf(out, 2, "title", &[], if opts.keep_title_numbers { &i.raw_title } else { &i.title });

    // XMLTV allows <title> to repeat. The data has no language field for the original title,
    // so no lang attribute is written (it is optional) rather than guessing one.
    if let Some(original) = &i.original_title {
        leaf(out, 2, "title", &[], original);
    }

    if let Some(sub_title) = &i.sub_title {
        leaf(out, 2, "sub-title", &[], sub_title);
    }
    if let Some(description) = &i.description {
        leaf(out, 2, "desc", &[], description);
    }

    if !(i.directors.is_empty()
        && i.actors.is_empty()
        && i.writers.is_empty()
        && i.producers.is_empty()
        && i.composers.is_empty()
        && i.presenters.is_empty())
    {
        open(out, 2, "credits", &[]);
        for n in &i.directors {
            person(out, 3, "director", n);
        }
        for n in &i.actors {
            person(out, 3, "actor", n);
        }
        for n in &i.writers {
            person(out, 3, "writer", n);
        }
        for n in &i.producers {
            person(out, 3, "producer", n);
        }
        for n in &i.composers {
            person(out, 3, "composer", n);
        }
        for n in &i.presenters {
            person(out, 3, "presenter", n);
        }
        close(out, 2, "credits");
    }

    if let Some(year) = i.year {
        leaf(out, 2, "date", &[], &year.to_string());
    }

    for c in &i.categories {
        leaf(out, 2, "category", &[("lang", c.lang)], &c.text);
    }

    if let Some(minutes) = i.length_minutes {
        leaf(out, 2, "length", &[("units", "minutes")], &minutes.to_string());
    }

    if !opts.no_images {
        if let Some(icon) = &i.icon {
            empty(out, 2, "icon", &[("src", icon.as_str())]);
        }
    }

    if let Some(country) = &i.country {
        leaf(out, 2, "country", &[("lang", "cs")], country);
    }

    if let Some(e) = &i.episode {
        leaf(out, 2, "episode-num", &[("system", "xmltv_ns")], &to_xmltv_ns(e));
        leaf(out, 2, "episode-num", &[("system", "onscreen")], &to_on_screen(e));
    }

    if i.black_and_white || i.aspect.is_some() || i.quality.is_some() {
        open(out, 2, "video", &[]);
        if i.black_and_white {
            leaf(out, 3, "colour", &[], "no");
        }
        if let Some(aspect) = i.aspect {
            leaf(out, 3, "aspect", &[], aspect);
        }
        if let Some(quality) = i.quality {
            leaf(out, 3, "quality", &[], quality);
        }
        close(out, 2, "video");
    }

    if let Some(audio) = i.audio {
        open(out, 2, "audio", &[]);
        leaf(out, 3, "stereo", &[], audio);
        close(out, 2, "audio");
    }

    if i.previously_shown {
        empty(out, 2, "previously-shown", &[]);
    }
    if i.premiere {
        empty(out, 2, "premiere", &[]);
    }

    for kind in i.subtitle_types.iter().copied() {
        empty(out, 2, "subtitles", &[("type", kind)]);
    }

    if let Some(rating) = &i.rating {
        open(out, 2, "star-rating", &[("system", "sms.cz")]);
        leaf(out, 3, "value", &[], rating);
        close(out, 2, "star-rating");
    }

    close(out, 1, "programme");
}

// ---------- the final file ----------

/// Writes the final file: header, the channels that have programmes, then the already-serialised
/// programmes copied from the temporary file. Nothing is held in memory.
///
/// The file is written under a temporary name first so a crash never leaves a half-written file behind.
pub fn assemble_output(full: &Path, tmp: &Path, channels: &[Arc<Channel>], programmes_tmp: &Path) -> io::Result<()> {
    let gzip = full.to_string_lossy().to_lowercase().ends_with(".gz");
    let file = File::create(tmp)?;

    if gzip {
        let mut encoder = GzEncoder::new(BufWriter::new(file), Compression::default());
        write_document(&mut encoder, channels, programmes_tmp)?;
        encoder.finish()?.flush()?;
    } else {
        let mut writer = BufWriter::new(file);
        write_document(&mut writer, channels, programmes_tmp)?;
        writer.flush()?;
    }

    fs::rename(tmp, full)
}

fn write_document<W: Write>(w: &mut W, channels: &[Arc<Channel>], programmes_tmp: &Path) -> io::Result<()> {
    w.write_all(b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n")?;
    w.write_all(b"<tv generator-info-name=\"rxmltv\" generator-info-url=\"http://www.somesite.eu/\">\n")?;

    let mut buffer = String::new();
    for ch in channels {
        buffer.clear();
        write_channel(&mut buffer, ch);
        w.write_all(buffer.as_bytes())?;
    }

    let mut programmes = File::open(programmes_tmp)?;
    io::copy(&mut programmes, w)?; // every <programme>

    w.write_all(b"</tv>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_attribute_escaping() {
        let mut out = String::new();
        push_escaped(&mut out, "a<b>&\"c\"\n", false);
        assert_eq!(out, "a&lt;b&gt;&amp;\"c\"\n");

        let mut out = String::new();
        push_escaped(&mut out, "a<b>&\"c\"\n", true);
        assert_eq!(out, "a&lt;b&gt;&amp;&quot;c&quot;&#xA;");
    }

    #[test]
    fn illegal_xml_characters_are_dropped() {
        let mut out = String::new();
        push_escaped(&mut out, "ok\u{0}\u{8}\u{FFFE}!", false);
        assert_eq!(out, "ok!");
    }

    #[test]
    fn channel_layout() {
        let ch = Channel::new("2".into(), "ČT 1".into(), Some("ct1.png".into()), "cz".into());
        let mut out = String::new();
        write_channel(&mut out, &ch);
        assert_eq!(
            out,
            "  <channel id=\"cz-ct-1-2\">\n    <display-name lang=\"cs\">ČT 1</display-name>\n    <icon src=\"https://www.sms.cz/kategorie/televize/bmp/loga/velka/ct1.png\" />\n  </channel>\n"
        );
    }
}
