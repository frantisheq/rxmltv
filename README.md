| `--clean-cache` | Delete cached files older than today, expired "no data" markers and leftover temp files |
| `--concurrency <n>` | Parallel downloads (default 4) |
| `--groups <a,b,c>` | Only these channel groups, by name or code, e.g. `"České,Slovenské"` or `cz,sk`. Default: all groups |
| `--channels <a,b,c>` | Pick individual channels by XMLTV id, e.g. `cz-ct1-2,sk-markiza-7` (see `--list-channels`). Combined with `--groups`, you get both selections |
| `--channels-file <file>` | Same, one id per line; lines starting with `#` are ignored and only the first word of a line is used, so you can paste lines from `--list-channels` as they are. Can be repeated |
| `--list-channels` | Print every channel (XMLTV id and name) so you know what to put in `--channels`, then exit |
| `--list-groups` | Print the groups the server offers with channel counts, then exit |
| `--keep-title-numbers` | Keep `(3)`, `(2/6)` and roman season numbers in the title. Default: removed and written as `<episode-num>` |
cz-ct1-2
cz-nova-5     Nova
sk-markiza-7
```

Run it once or twice a day. For example with cron:

```
## Releases (maintainers)

GitHub Actions (`.github/workflows/build.yml`) builds for Linux, FreeBSD, Windows and macOS on every push to
`master`. To publish a release, bump `version` in `Cargo.toml`, then:

```sh
git tag v1.0.0
