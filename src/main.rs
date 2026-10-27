mod stats;
mod util;

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
  --clean-cache       Delete cached files older than today
  --concurrency <n>   Parallel downloads (default 4)
  --groups <a,b,c>    Only these channel groups, by name or code, e.g. "České,Slovenské" or "cz,sk" (default: all groups)
  --channels <a,b,c>  Pick individual channels by their XMLTV id, e.g. "cz-ct1-2,sk-markiza-7"
                      (see --list-channels). Combined with --groups, the result is both selections together
  --channels-file <f> Same as --channels, one id per line ("#" starts a comment, only the first word of a line is used)
  --list-groups       Print the groups the server offers, with channel counts, and exit
  --list-channels     Print every channel (XMLTV id and name) for use with --channels, and exit
  --keep-title-numbers   Leave "(3)", "(2/6)" and roman season numbers in the title
    pub cache_dir: String,
    pub concurrency: usize,
    pub groups: Option<Vec<String>>,
    pub channels: Vec<String>,
    pub list_groups: bool,
    pub list_channels: bool,
    pub keep_title_numbers: bool,
    pub retry_missing: bool,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
            }
            "--channels" => {
                let v = take_value(name, inline, args, &mut i)?;
                o.channels.extend(v.split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from));
            }
            "--channels-file" => {
                let path = take_value(name, inline, args, &mut i)?;
                let text = fs::read_to_string(&path).map_err(|e| format!("Cannot read {path}: {e}"))?;
                o.channels.extend(
                    text.trim_start_matches('\u{feff}')
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.starts_with('#'))
                        .filter_map(|l| l.split_whitespace().next())
                        .map(String::from),
                );
            }
            "--days" => o.days = parse_positive(name, &take_value(name, inline, args, &mut i)?)?,
            "--concurrency" => {

/// Which channels to produce: everything when no filter is given, otherwise the channels of the
/// requested groups plus the individually requested ones (by XMLTV id, ignoring case).
fn parse_channels(raw: &[RawChannel], groups: Option<&[String]>, channels: &[String]) -> Vec<Arc<Channel>> {
    let wanted_groups: Option<HashSet<String>> = groups.map(|g| g.iter().map(|x| normalize(x)).collect());
    let wanted_ids: HashSet<String> = channels.iter().map(|c| c.trim().to_lowercase()).collect();
    let filtering = wanted_groups.is_some() || !wanted_ids.is_empty();
    let mut found: HashSet<String> = HashSet::new();

            continue;
        }
        let code = group_code(Some(&r.group));
        let channel = Channel::new(id.clone(), name.clone(), r.logo.clone(), code.clone());

        if filtering {
            let group_n = normalize(&r.group);
                .as_ref()
                .is_some_and(|w| w.contains(&group_n) || w.contains(&code));

            let xml_id = channel.xml_id.to_lowercase();
            let picked = wanted_ids.contains(&xml_id);
            if picked {
                found.insert(xml_id);
            }
            if !(in_group || picked) {
                continue;
            }
        }
        items.push((channel, code, unidecode(name)));
    }

    for wanted in channels {
        if !found.contains(&wanted.trim().to_lowercase()) {
            eprintln!("Warning: channel \"{wanted}\" not found (see --list-channels).");
        }
    }

        return Ok(1);
    }

    eprintln!("{} channels, {} day(s).", channels.len(), opts.days);

    let clock = Instant::now();
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_filter_selects_everything() {
        assert_eq!(ids(&parse_channels(&sample(), None, &[])), vec!["1", "2", "4", "3"]);

    #[test]
    fn channels_are_picked_by_xmltv_id() {
        assert_eq!(ids(&parse_channels(&sample(), None, &list(&["cz-nova-2"]))), vec!["2"]);
        assert_eq!(
            ids(&parse_channels(&sample(), None, &list(&["sk-markiza-3", "cz-ct1-1"]))),
            vec!["1", "3"]
        );
    }

    #[test]
    fn xmltv_ids_ignore_case() {
        assert_eq!(ids(&parse_channels(&sample(), None, &list(&["SK-Markiza-3"]))), vec!["3"]);
    }

    #[test]
    fn names_and_plain_ids_do_not_match() {
        assert!(parse_channels(&sample(), None, &list(&["Nova"])).is_empty());
        assert!(parse_channels(&sample(), None, &list(&["2"])).is_empty());
    }

    #[test]
    fn groups_and_channels_are_combined() {
        let groups = list(&["sk"]);
        let chosen = parse_channels(&sample(), Some(&groups), &list(&["cz-nova-2"]));
        assert_eq!(ids(&chosen), vec!["2", "3"]);
    }

    #[test]
    fn channels_option_is_parsed() {
        let o = parse_args(&args(&["--channels", "cz-nova-2, sk-markiza-3"])).unwrap().unwrap();
        assert_eq!(o.channels, vec!["cz-nova-2".to_string(), "sk-markiza-3".to_string()]);
    }

    #[test]
        fs::write(&path, "# my channels\ncz-nova-2   Nova\n\n  sk-markiza-3\n").unwrap();
        let o = parse_args(&args(&["--channels-file", path.to_str().unwrap()])).unwrap().unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(o.channels, vec!["cz-nova-2".to_string(), "sk-markiza-3".to_string()]);
    }

    #[test]
