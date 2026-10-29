//! Small helpers shared by the other modules.

use std::path::{Path, PathBuf};

use roxmltree::Node;

// ---------- XML reading (the equivalent of XElement.Element / .Value) ----------

pub fn is_el(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

/// First direct child element with this name.
pub fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is_el(c, name))
}

/// All text below the node, concatenated (XElement.Value).
pub fn value(n: Node) -> String {
    n.descendants()
        .filter_map(|d| if d.is_text() { d.text() } else { None })
        .collect()
}

pub fn child_value(n: Node, name: &str) -> Option<String> {
    child(n, name).map(value)
}

// ---------- text ----------

/// The server escapes some text twice, so "&amp;#8222;" and "&amp;ocirc;" survive XML parsing.
pub fn html_decode(s: &str) -> String {
    html_escape::decode_html_entities(s).into_owned()
}

/// Trimmed text, or None when nothing is left.
pub fn blank(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn opt_blank(s: Option<String>) -> Option<String> {
    s.and_then(|s| blank(&s))
}

/// Only the ASCII digits of the input.
pub fn get_numbers(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn count_letters(s: &str) -> usize {
    s.chars().filter(|c| c.is_alphabetic()).count()
}

/// Letters and digits only, lower case: a key for "is this the same text?" comparisons.
pub fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

// ---------- files ----------

/// "file.xml" + ".miss" -> "file.xml.miss"
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(|name| std::env::var_os(name))
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// A path as typed in the settings: a leading `~` means the home folder.
pub fn expand_home(text: &str) -> PathBuf {
    let text = text.trim();
    if text == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(text));
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

pub fn try_delete(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Deletes a file when dropped, so a crash or Ctrl+C never leaves temporary files behind.
pub struct TempGuard(pub PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        try_delete(&self.0);
    }
}
