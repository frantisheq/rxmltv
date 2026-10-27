use std::cmp::Ordering;

use deunicode::deunicode_with_tofu;

pub const OTHERS: &str = "others";

/// ASCII transliteration (like Unidecode.NET): "České" -> "Ceske". Characters with no mapping are dropped.
pub fn unidecode(s: &str) -> String {
    deunicode_with_tofu(s, "")
}

/// Case- and diacritics-insensitive, so "ceske" matches "České".
pub fn normalize(s: &str) -> String {
    unidecode(s).trim().to_lowercase()
}

/// Lower-case ASCII letters and digits only.
fn group_key(group: &str) -> String {
    unidecode(group)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn known_code(key: &str) -> Option<&'static str> {
    Some(match key {
        "ceske" => "cz",
        "slovenske" => "sk",
        "anglicke" => "en",
        "nemecke" => "de",
        "polske" => "pl",
        "francouzske" => "fr",
        "madarske" => "hu",
        "italske" => "it",
        "ruske" => "ru",
        "spanelske" => "es",
        "ostatni" => OTHERS,
        _ => return None,
    })
}

/// Turns the server's channel groups ("České", "Ruské", "Ostatní") into short codes.
/// An unknown group keeps its own name (ASCII, lower case); no group is "others".
pub fn group_code(group: Option<&str>) -> String {
    let key = group_key(group.unwrap_or(""));
    if key.is_empty() {
        return OTHERS.to_string();
    }
    match known_code(&key) {
        Some(code) => code.to_string(),
        None => key,
    }
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn trim_zeros(digits: &[char]) -> &[char] {
    let first = digits.iter().position(|&c| c != '0').unwrap_or(digits.len());
    &digits[first..]
}

/// Case-insensitive ordering where numbers compare by value: "Sport 2" comes before "Sport 10".
pub fn natural_cmp(x: &str, y: &str) -> Ordering {
    let x: Vec<char> = x.chars().collect();
    let y: Vec<char> = y.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);

    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let (start_x, start_y) = (i, j);
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }

            // Compare the two numbers by value without parsing them (leading zeros ignored).
            let a = trim_zeros(&x[start_x..i]);
            let b = trim_zeros(&y[start_y..j]);
            if a.len() != b.len() {
                return a.len().cmp(&b.len());
            }
            let by_digits = a.cmp(b);
            if by_digits != Ordering::Equal {
                return by_digits;
            }
        } else {
            let by_char = lower(x[i]).cmp(&lower(y[j]));
            if by_char != Ordering::Equal {
                return by_char;
            }
            i += 1;
            j += 1;
        }
    }

    (x.len() - i).cmp(&(y.len() - j))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_groups_become_codes() {
        assert_eq!(group_code(Some("České")), "cz");
        assert_eq!(group_code(Some("Slovenské")), "sk");
        assert_eq!(group_code(Some("Ostatní")), "others");
    }

    #[test]
    fn unknown_or_missing_group() {
        assert_eq!(group_code(Some("Švédské")), "svedske");
        assert_eq!(group_code(Some("   ")), "others");
        assert_eq!(group_code(None), "others");
    }

    #[test]
    fn normalize_ignores_case_and_diacritics() {
        assert_eq!(normalize("  České "), "ceske");
    }

    #[test]
    fn numbers_compare_by_value() {
        assert_eq!(natural_cmp("Sport 2", "Sport 10"), Ordering::Less);
        assert_eq!(natural_cmp("Sport 10", "Sport 2"), Ordering::Greater);
        assert_eq!(natural_cmp("sport", "SPORT"), Ordering::Equal);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Equal);
        assert_eq!(natural_cmp("ct1", "ct24"), Ordering::Less);
    }
}
