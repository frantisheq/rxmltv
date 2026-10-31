//! Genre names: capitalising them, and English translations of the Czech ones the server uses.

use crate::enrichment::Category;
use crate::groups::normalize;

/// "akční" becomes "Akční".
pub fn capitalize_first(text: &str) -> String {
    let mut chars = text.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The English name of a Czech genre (matched without diacritics, any case), if the list below has it.
pub fn english(czech: &str) -> Option<&'static str> {
    let key = normalize(czech);
    let name = match key.as_str() {
        "akcni" => "Action",
        "dobrodruzny" => "Adventure",
        "drama" | "dramaticky" => "Drama",
        "komedie" => "Comedy",
        "romanticky" | "romance" => "Romance",
        "thriller" | "psychothriller" => "Thriller",
        "horor" => "Horror",
        "kriminalni" | "krimi" => "Crime",
        "detektivni" => "Detective",
        "mysteriozni" | "mysticky" => "Mystery",
        "sci-fi" | "scifi" | "sci fi" | "vedecko-fantasticky" => "Sci-fi",
        "fantasy" | "fantasticky" => "Fantasy",
        "western" => "Western",
        "valecny" | "vojensky" => "War",
        "historicky" | "historie" => "Historical",
        "dobovy" => "Period",
        "zivotopisny" | "biograficky" => "Biography",
        "hudebni" | "hudba" => "Music",
        "muzikal" => "Musical",
        "animovany" | "animace" => "Animated",
        "rodinny" => "Family",
        "pohadka" | "pohadkovy" => "Fairy tale",
        "detsky" | "pro deti" | "deti" => "Children's",
        "sportovni" | "sport" => "Sports",
        "zpravodajstvi" | "zpravy" | "zpravodajsky" => "News",
        "publicistika" | "publicisticky" => "Current affairs",
        "magazin" => "Magazine",
        "talk show" | "talkshow" => "Talk show",
        "soutez" | "soutezni" | "kviz" => "Game show",
        "reality show" | "reality" => "Reality show",
        "vzdelavaci" | "naucny" | "osvetovy" => "Educational",
        "cestopisny" | "cestovani" => "Travel",
        "prirodopisny" | "priroda" => "Nature",
        "zvirata" | "zviratka" => "Animals",
        "dokumentarni" | "dokument" => "Documentary",
        "nabozensky" => "Religious",
        "eroticky" => "Erotic",
        "psychologicky" => "Psychological",
        "katastroficky" => "Disaster",
        "gangstersky" => "Gangster",
        "politicky" => "Political",
        "spolecensky" => "Social",
        "zdravotnicky" | "lekarsky" => "Medical",
        "kulinarsky" | "vareni" => "Cooking",
        "zivotni styl" | "lifestyle" => "Lifestyle",
        "zabavny" | "zabava" => "Entertainment",
        "satiricky" | "satira" => "Satire",
        "parodie" => "Parody",
        "film" | "filmovy" => "Movie",
        "serial" => "Series",
        "telenovela" => "Telenovela",
        "tanecni" | "tanec" => "Dance",
        "opera" => "Opera",
        "koncert" => "Concert",
        "divadlo" | "divadelni" => "Theatre",
        "umeni" => "Arts",
        "kultura" | "kulturni" => "Culture",
        "veda" | "vedecky" => "Science",
        "technika" => "Technology",
        "motorismus" | "auto-moto" => "Motoring",
        "hobby" => "Hobbies",
        "zahradnictvi" | "zahrada" => "Gardening",
        "moda" => "Fashion",
        "ekonomika" | "ekonomicky" => "Economy",
        "anime" => "Anime",
        "komiksovy" | "komiks" => "Comics",
        "gameshow" => "Game show",
        "groteskni" | "groteska" => "Grotesque",
        "noir" | "film noir" => "Film noir",
        _ => return None,
    };
    Some(name)
}

/// The categories to write: every one with its first letter capitalised, and (when `translate` is on) after each
/// non-English one its English name, unless that is already among the categories.
pub fn category_elements(categories: &[Category], translate: bool) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let present = |out: &[(&'static str, String)], text: &str| out.iter().any(|(_, t)| t.eq_ignore_ascii_case(text));

    for category in categories {
        let text = capitalize_first(&category.text);
        if text.is_empty() || present(&out, &text) {
            continue;
        }
        out.push((category.lang, text));

        if translate && category.lang != "en" {
            if let Some(name) = english(&category.text) {
                let already = categories.iter().any(|c| c.text.eq_ignore_ascii_case(name));
                if !already && !present(&out, name) {
                    out.push(("en", name.to_string()));
                }
            }
        }
    }
    out
}

/// For reading a guide back: the categories (language, text) without the English ones that are only the
/// translation of another category in the list.
pub fn without_translations(all: &[(String, String)]) -> Vec<String> {
    all.iter()
        .filter(|(lang, text)| {
            !(lang == "en"
                && all
                    .iter()
                    .any(|(l, t)| l != "en" && english(t).is_some_and(|name| name.eq_ignore_ascii_case(text))))
        })
        .map(|(_, text)| text.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(lang: &'static str, text: &str) -> Category {
        Category { lang, text: text.to_string() }
    }

    #[test]
    fn first_letters_are_capitalised() {
        assert_eq!(capitalize_first("akční"), "Akční");
        assert_eq!(capitalize_first(" drama "), "Drama");
        assert_eq!(capitalize_first("čeština"), "Čeština");
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn czech_genres_have_english_names() {
        assert_eq!(english("akční"), Some("Action"));
        assert_eq!(english("Mysteriózní"), Some("Mystery"));
        assert_eq!(english("DOBRODRUŽNÝ"), Some("Adventure"));
        assert_eq!(english("neznámý"), None);
    }

    #[test]
    fn translations_follow_their_czech_genre() {
        let categories = vec![cat("en", "Series"), cat("cs", "akční"), cat("cs", "dobrodružný"), cat("cs", "něco jiného")];
        let out = category_elements(&categories, true);
        let texts: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["Series", "Akční", "Action", "Dobrodružný", "Adventure", "Něco jiného"]);
        assert_eq!(out[2].0, "en");
        assert_eq!(out[1].0, "cs");
    }

    #[test]
    fn translations_can_be_switched_off() {
        let categories = vec![cat("cs", "drama")];
        let out = category_elements(&categories, false);
        assert_eq!(out, vec![("cs", "Drama".to_string())]);
    }

    #[test]
    fn a_translation_is_not_added_twice() {
        // "Documentary" is already there (from the server's type), so "dokumentární" adds nothing
        let categories = vec![cat("en", "Documentary"), cat("cs", "dokumentární")];
        let out = category_elements(&categories, true);
        assert_eq!(out.len(), 2);
        let texts: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["Documentary", "Dokumentární"]);
    }

    #[test]
    fn written_guides_read_back_without_the_translations() {
        let all = vec![
            ("en".to_string(), "Movie".to_string()),
            ("cs".to_string(), "Drama".to_string()),
            ("en".to_string(), "Drama".to_string()), // the translation of the Czech one
            ("cs".to_string(), "Mysteriózní".to_string()),
            ("en".to_string(), "Mystery".to_string()),
        ];
        assert_eq!(without_translations(&all), vec!["Movie", "Drama", "Mysteriózní"]);
    }
}
