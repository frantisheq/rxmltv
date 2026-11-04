//! Genre names: capitalising them, and English translations of the Czech ones the server uses.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::enrichment::Category;
use crate::groups::normalize;
use crate::i18n::Lang;

/// "akční" becomes "Akční".
pub fn capitalize_first(text: &str) -> String {
    let mut chars = text.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A genre in the three languages. `keys` are the Czech names as the server writes them, in the normalized form
/// (lower case, no diacritics), including a few alternative forms.
struct Genre {
    keys: &'static [&'static str],
    en: &'static str,
    sk: &'static str,
    cs: &'static str,
}

const GENRES: &[Genre] = &[
    Genre { keys: &["akcni"], en: "Action", sk: "Akčný", cs: "Akční" },
    Genre { keys: &["dobrodruzny"], en: "Adventure", sk: "Dobrodružný", cs: "Dobrodružný" },
    Genre { keys: &["drama", "dramaticky"], en: "Drama", sk: "Dráma", cs: "Drama" },
    Genre { keys: &["komedie"], en: "Comedy", sk: "Komédia", cs: "Komedie" },
    Genre { keys: &["romanticky", "romance"], en: "Romance", sk: "Romantický", cs: "Romantický" },
    Genre { keys: &["thriller", "psychothriller"], en: "Thriller", sk: "Triler", cs: "Thriller" },
    Genre { keys: &["horor"], en: "Horror", sk: "Horor", cs: "Horor" },
    Genre { keys: &["kriminalni", "krimi"], en: "Crime", sk: "Kriminálny", cs: "Kriminální" },
    Genre { keys: &["detektivni"], en: "Detective", sk: "Detektívny", cs: "Detektivní" },
    Genre { keys: &["mysteriozni", "mysticky"], en: "Mystery", sk: "Mysteriózny", cs: "Mysteriózní" },
    Genre { keys: &["sci-fi", "scifi", "sci fi", "vedecko-fantasticky"], en: "Sci-fi", sk: "Sci-fi", cs: "Sci-fi" },
    Genre { keys: &["fantasy", "fantasticky"], en: "Fantasy", sk: "Fantasy", cs: "Fantasy" },
    Genre { keys: &["western"], en: "Western", sk: "Western", cs: "Western" },
    Genre { keys: &["valecny", "vojensky"], en: "War", sk: "Vojnový", cs: "Válečný" },
    Genre { keys: &["historicky", "historie"], en: "Historical", sk: "Historický", cs: "Historický" },
    Genre { keys: &["dobovy"], en: "Period", sk: "Dobový", cs: "Dobový" },
    Genre { keys: &["zivotopisny", "biograficky"], en: "Biography", sk: "Životopisný", cs: "Životopisný" },
    Genre { keys: &["hudebni", "hudba"], en: "Music", sk: "Hudobný", cs: "Hudební" },
    Genre { keys: &["muzikal"], en: "Musical", sk: "Muzikál", cs: "Muzikál" },
    Genre { keys: &["animovany", "animace"], en: "Animated", sk: "Animovaný", cs: "Animovaný" },
    Genre { keys: &["rodinny"], en: "Family", sk: "Rodinný", cs: "Rodinný" },
    Genre { keys: &["pohadka", "pohadkovy"], en: "Fairy tale", sk: "Rozprávka", cs: "Pohádka" },
    Genre { keys: &["detsky", "pro deti", "deti"], en: "Children's", sk: "Detský", cs: "Dětský" },
    Genre { keys: &["sportovni", "sport"], en: "Sports", sk: "Športový", cs: "Sportovní" },
    Genre { keys: &["zpravodajstvi", "zpravy", "zpravodajsky"], en: "News", sk: "Spravodajstvo", cs: "Zpravodajství" },
    Genre { keys: &["publicistika", "publicisticky"], en: "Current affairs", sk: "Publicistika", cs: "Publicistika" },
    Genre { keys: &["magazin"], en: "Magazine", sk: "Magazín", cs: "Magazín" },
    Genre { keys: &["talk show", "talkshow"], en: "Talk show", sk: "Talk show", cs: "Talk show" },
    Genre { keys: &["soutez", "soutezni", "kviz", "gameshow"], en: "Game show", sk: "Súťaž", cs: "Soutěž" },
    Genre { keys: &["reality show", "reality"], en: "Reality show", sk: "Reality show", cs: "Reality show" },
    Genre { keys: &["vzdelavaci", "naucny", "osvetovy"], en: "Educational", sk: "Vzdelávací", cs: "Vzdělávací" },
    Genre { keys: &["cestopisny", "cestovani"], en: "Travel", sk: "Cestopisný", cs: "Cestopisný" },
    Genre { keys: &["prirodopisny", "priroda"], en: "Nature", sk: "Prírodopisný", cs: "Přírodopisný" },
    Genre { keys: &["zvirata", "zviratka"], en: "Animals", sk: "Zvieratá", cs: "Zvířata" },
    Genre { keys: &["dokumentarni", "dokument"], en: "Documentary", sk: "Dokumentárny", cs: "Dokumentární" },
    Genre { keys: &["nabozensky"], en: "Religious", sk: "Náboženský", cs: "Náboženský" },
    Genre { keys: &["eroticky"], en: "Erotic", sk: "Erotický", cs: "Erotický" },
    Genre { keys: &["psychologicky"], en: "Psychological", sk: "Psychologický", cs: "Psychologický" },
    Genre { keys: &["katastroficky"], en: "Disaster", sk: "Katastrofický", cs: "Katastrofický" },
    Genre { keys: &["gangstersky"], en: "Gangster", sk: "Gangsterský", cs: "Gangsterský" },
    Genre { keys: &["politicky"], en: "Political", sk: "Politický", cs: "Politický" },
    Genre { keys: &["spolecensky"], en: "Social", sk: "Spoločenský", cs: "Společenský" },
    Genre { keys: &["zdravotnicky", "lekarsky"], en: "Medical", sk: "Zdravotnícky", cs: "Zdravotnický" },
    Genre { keys: &["kulinarsky", "vareni"], en: "Cooking", sk: "Kulinársky", cs: "Kulinářský" },
    Genre { keys: &["zivotni styl", "lifestyle"], en: "Lifestyle", sk: "Životný štýl", cs: "Životní styl" },
    Genre { keys: &["zabavny", "zabava"], en: "Entertainment", sk: "Zábavný", cs: "Zábavný" },
    Genre { keys: &["satiricky", "satira"], en: "Satire", sk: "Satirický", cs: "Satirický" },
    Genre { keys: &["parodie"], en: "Parody", sk: "Paródia", cs: "Parodie" },
    Genre { keys: &["film", "filmovy"], en: "Movie", sk: "Film", cs: "Film" },
    Genre { keys: &["serial"], en: "Series", sk: "Seriál", cs: "Seriál" },
    Genre { keys: &["telenovela"], en: "Telenovela", sk: "Telenovela", cs: "Telenovela" },
    Genre { keys: &["tanecni", "tanec"], en: "Dance", sk: "Tanečný", cs: "Taneční" },
    Genre { keys: &["opera"], en: "Opera", sk: "Opera", cs: "Opera" },
    Genre { keys: &["koncert"], en: "Concert", sk: "Koncert", cs: "Koncert" },
    Genre { keys: &["divadlo", "divadelni"], en: "Theatre", sk: "Divadlo", cs: "Divadlo" },
    Genre { keys: &["umeni"], en: "Arts", sk: "Umenie", cs: "Umění" },
    Genre { keys: &["kultura", "kulturni"], en: "Culture", sk: "Kultúra", cs: "Kultura" },
    Genre { keys: &["veda", "vedecky"], en: "Science", sk: "Veda", cs: "Věda" },
    Genre { keys: &["technika"], en: "Technology", sk: "Technika", cs: "Technika" },
    Genre { keys: &["motorismus", "auto-moto"], en: "Motoring", sk: "Motorizmus", cs: "Motorismus" },
    Genre { keys: &["hobby"], en: "Hobbies", sk: "Hobby", cs: "Hobby" },
    Genre { keys: &["zahradnictvi", "zahrada"], en: "Gardening", sk: "Záhradníctvo", cs: "Zahradnictví" },
    Genre { keys: &["moda"], en: "Fashion", sk: "Móda", cs: "Móda" },
    Genre { keys: &["ekonomika", "ekonomicky"], en: "Economy", sk: "Ekonomika", cs: "Ekonomika" },
    Genre { keys: &["anime"], en: "Anime", sk: "Anime", cs: "Anime" },
    Genre { keys: &["komiksovy", "komiks"], en: "Comics", sk: "Komiksový", cs: "Komiksový" },
    Genre { keys: &["groteskni", "groteska"], en: "Grotesque", sk: "Groteska", cs: "Groteska" },
    Genre { keys: &["noir", "film noir"], en: "Film noir", sk: "Film noir", cs: "Film noir" },
];

/// Every spelling (normalized) of every genre -> its entry.
fn index() -> &'static HashMap<String, &'static Genre> {
    static MAP: OnceLock<HashMap<String, &'static Genre>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map: HashMap<String, &'static Genre> = HashMap::new();
        for genre in GENRES {
            let names = genre.keys.iter().copied().chain([genre.en, genre.sk, genre.cs]);
            for name in names {
                map.entry(normalize(name)).or_insert(genre);
            }
        }
        map
    })
}

fn find(text: &str) -> Option<&'static Genre> {
    index().get(&normalize(text)).copied()
}

/// The English name of a genre (given in Czech, Slovak or English), if the list above has it.
pub fn english(text: &str) -> Option<&'static str> {
    find(text).map(|g| g.en)
}

/// The name of a genre in the given language; a genre that is not in the list is only capitalised.
pub fn localize(text: &str, lang: Lang) -> String {
    match find(text) {
        Some(genre) => match lang {
            Lang::En => genre.en,
            Lang::Cs => genre.cs,
            Lang::Sk => genre.sk,
        }
        .to_string(),
        None => capitalize_first(text),
    }
}

/// The normalized English and Slovak names of a genre, space separated ("" when it is not in the list). They
/// are added to what the genre colour rules match on, so a keyword works in any of the languages.
pub fn matching_names(text: &str) -> String {
    match find(text) {
        Some(genre) => format!("{} {}", normalize(genre.en), normalize(genre.sk)),
        None => String::new(),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The Slovak name of a genre (given in Czech, Slovak or English), if the list above has it.
pub fn slovak(text: &str) -> Option<&'static str> {
    find(text).map(|g| g.sk)
}

/// The categories to write: every one with its first letter capitalised, and after each one its English name
/// (when `english_on`) and its Slovak name (when `slovak_on`), unless that is already among the categories.
pub fn category_elements(categories: &[Category], english_on: bool, slovak_on: bool) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let present = |out: &[(&'static str, String)], text: &str| out.iter().any(|(_, t)| same(t, text));
    let given = |text: &str| categories.iter().any(|c| same(&c.text, text));

    for category in categories {
        let text = capitalize_first(&category.text);
        if text.is_empty() || present(&out, &text) {
            continue;
        }
        out.push((category.lang, text));

        let Some(genre) = find(&category.text) else { continue };
        if english_on && category.lang != "en" && !given(genre.en) && !present(&out, genre.en) {
            out.push(("en", genre.en.to_string()));
        }
        if slovak_on && category.lang != "sk" && !given(genre.sk) && !present(&out, genre.sk) {
            out.push(("sk", genre.sk.to_string()));
        }
    }
    out
}

/// For reading a guide back: the categories (language, text) without the English and Slovak ones that are only
/// the translation of another category in the list.
pub fn without_translations(all: &[(String, String)]) -> Vec<String> {
    all.iter()
        .filter(|(lang, text)| {
            // An English name translates a Czech one; a Slovak name translates a Czech or an English one (the
            // server's English types get a Slovak name). Never the other way round, or two would cancel out.
            let translation = |kind: &str, name_of: fn(&str) -> Option<&'static str>| {
                lang == kind
                    && all.iter().any(|(l, t)| {
                        let original = if kind == "en" { l != "en" && l != "sk" } else { l != "sk" };
                        original && name_of(t).is_some_and(|n| same(n, text))
                    })
            };
            !(translation("en", english) || translation("sk", slovak))
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
    fn genres_are_shown_in_the_chosen_language() {
        assert_eq!(localize("Akční", Lang::En), "Action");
        assert_eq!(localize("akční", Lang::Cs), "Akční");
        assert_eq!(localize("Akční", Lang::Sk), "Akčný");
        // the server's English types work too
        assert_eq!(localize("Movie", Lang::Cs), "Film");
        assert_eq!(localize("Series", Lang::Sk), "Seriál");
        assert_eq!(localize("Documentary", Lang::Cs), "Dokumentární");
        // an unknown genre is only capitalised
        assert_eq!(localize("něco jiného", Lang::En), "Něco jiného");
    }

    #[test]
    fn names_in_every_language_are_matched() {
        let names = matching_names("sportovní");
        assert!(names.contains("sports") && names.contains("sportovy"));
        assert_eq!(matching_names("neznámý"), "");
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
        let out = category_elements(&categories, true, true);
        let texts: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        // "Dobrodružný" is the same word in Slovak, so it is not written twice
        assert_eq!(
            texts,
            vec!["Series", "Seriál", "Akční", "Action", "Akčný", "Dobrodružný", "Adventure", "Něco jiného"]
        );
        assert_eq!(out[1].0, "sk");
        assert_eq!(out[2].0, "cs");
        assert_eq!(out[3].0, "en");
        assert_eq!(out[4].0, "sk");
    }

    #[test]
    fn translations_can_be_switched_off() {
        let categories = vec![cat("cs", "drama")];
        let out = category_elements(&categories, false, false);
        assert_eq!(out, vec![("cs", "Drama".to_string())]);

        // each language can be switched off on its own
        let only_slovak = category_elements(&categories, false, true);
        assert_eq!(only_slovak, vec![("cs", "Drama".to_string()), ("sk", "Dráma".to_string())]);
        let only_english = category_elements(&categories, true, false);
        assert_eq!(only_english, vec![("cs", "Drama".to_string())]); // the English name is "Drama" too
    }

    #[test]
    fn a_translation_is_not_added_twice() {
        // "Documentary" is already there (from the server's type), so "dokumentární" adds nothing
        let categories = vec![cat("en", "Documentary"), cat("cs", "dokumentární")];
        let out = category_elements(&categories, true, false);
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
            ("sk".to_string(), "Mysteriózny".to_string()),
            ("sk".to_string(), "Film".to_string()), // the Slovak name of the English type "Movie"
        ];
        assert_eq!(without_translations(&all), vec!["Movie", "Drama", "Mysteriózní"]);
    }
}
