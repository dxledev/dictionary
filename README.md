# Dictionary

A small, portable dictionary for Linux. English works offline from an embedded
Webster's 1913; 23 Wiktionary editions work online. Colors are set from the
command line, saved to one JSON file, and an open window reloads them the
moment the file changes. It is a native Rust window with no web runtime.

## Features

- **Offline English.** 108,181 headwords from Webster's 1913, embedded in the
  binary and decompressed one letter at a time, so a lookup never holds a
  whole letter in memory.
- **23 Wiktionary editions** (Arabic to Vietnamese), chosen in the in-window
  language panel or with `--lang`. English falls back to Wiktionary when a
  word is missing from the offline text.
- **Urban Dictionary.** English lookups also show up to five Urban Dictionary
  definitions in their own section, fetched in parallel so they never delay
  the dictionary result. It also fills in slang that Webster's and Wiktionary
  lack. Entries can be explicit; **Ctrl+U** hides the
  section. This uses Urban Dictionary's unofficial public JSON endpoint, whose
  terms say API access needs their permission, so check that before
  redistributing the app.
- **Heuristic non-English parsing.** English Wiktionary entries come from its
  structured definitions API. Other editions are read from page text, whose
  layout differs per edition, so senses are best-effort: French, German,
  Spanish, Italian, Turkish and Japanese read well; some editions (for
  example Russian and Polish) may show extra sections or nothing.
- **"Did you mean"** chips: up to three close words when nothing matches.
- **Themeable from the CLI** with live reload, including translucent
  backgrounds.
- **Resizable** native Wayland window. Panels (language, help) open *inside*
  the window over a softened copy of the page; there are no popup windows.
- **Icons are theme colored.** In-window icons are monochrome SVGs painted in
  the text or accent color. The launcher icon is a fixed, full-color picture
  of an open dictionary and does not follow the theme.

## Build and install

Requires Rust 1.97+, and `curl` at runtime for the online editions.

```sh
cargo build --release
scripts/install.sh              # ~/.local/bin, desktop entry, icon
scripts/install.sh --dry-run    # preview
```

`install.sh` accepts `--prefix`, `--config-dir`, `--apps-list` and
`--follow PALETTE` (see below). It installs the desktop entry named
"Dictionary" to `~/.local/share/applications` and `~/.local/bin/applications`,
and appends one line to an existing apps list if you keep one.

## Usage

```sh
dictionary                      # open the window
dictionary serendipity          # open and look a word up
dictionary --lang fr chat       # French edition
dictionary --selection          # look up the primary selection
dictionary --panel help         # open with a panel showing
```

`--selection` needs `wl-paste`, `xclip` or `xsel`; bind it to a key to look up
whatever is highlighted.

Keys: **Enter** looks up, **Esc** clears the field or closes a panel,
**Ctrl+L** chooses a language, **Ctrl+U** shows or hides Urban Dictionary,
**F1** shows help.

## Colors

The saved palette is `$XDG_CONFIG_HOME/dictionary/theme.json` (usually
`~/.config/dictionary/theme.json`). Every command below writes it atomically
and a running window reloads at once. `--config-dir /absolute/path` (before
the subcommand) uses another location.

```sh
dictionary theme show
dictionary theme set 'accent=#a9c7b0' 'background=#101820'
dictionary theme set 'background=#e00e1e36'
dictionary theme set 'background=Qt.rgba(14 / 255, 30 / 255, 54 / 255, 0.88)'
dictionary theme preset paper            # midnight paper forest rose mono
dictionary theme import /path/palette    # one-time copy
dictionary theme follow desktop /path/to/palette-dir-or-symlink
dictionary theme follow file /path/to/palette.json
dictionary theme follow off
dictionary theme sync                    # re-read the followed source now
dictionary theme reset
```

Add `--dry-run` to any change to preview without writing.

Tokens: `background surface surface_alt text muted accent accent_text border
danger warning success selection`. Setting `background`, `text` or `accent`
re-derives the colors built from them (surfaces, border, selection, and so
on); anything you name in the same command wins.

### Color formats

`#rrggbb`, `#rgb`, `#aarrggbb`, and `Qt.rgba(r, g, b, a)`.

Eight-digit colors use Qt's **#AARRGGBB**: alpha comes first. `#e00e1e36` is
the color `#0e1e36` at alpha `0xe0`; the RGB channels are never shifted.
`Qt.rgba` takes decimals or simple fractions. Only `background` carries
opacity. It is kept in `theme.json` and applied to the window; whether the
desktop shows through depends on the compositor, and where it does not, the
same RGB is drawn opaque.

### Following a palette

`theme follow` stores a source and keeps it current. A *palette directory*
holds a `Colors.qml` (properties such as `background`, `foreground`,
`primary`, `muted`, `success`, `warning`, `danger`) and/or a `palette.json`
(`base`, `text`, `primary`, `surface0`, `red`, ...). A symlink to one is
resolved on every check, so retargeting the link, or regenerating the files in
place, updates the window within a second. A palette that is half-written or a
link that dangles mid-switch is ignored and the last good colors stay. Running
`dictionary theme sync`, or opening the window, brings `theme.json` up to
date even if it was not running at the time. A manual `set`, `preset` or
`import` stops following.

Motion: panels fade and scale from 98% over 120 ms (ease-out-cubic). Set
`DICTIONARY_REDUCED_MOTION=1` to make them instant.

## Data and licenses

Code: MIT. The embedded Webster's 1913 data is derived from GCIDE and is
GPL-3.0-or-later (the 1913 text itself is public domain); see
`data/webster/LICENSE-DATA.txt`. Wiktionary content is available under
CC BY-SA. Urban Dictionary text belongs to its authors and is fetched at
lookup time, never stored. The common-word list used for suggestions is derived from Google's
10,000 most common English words.

The offline data was built from the GCIDE XML 0.53 release published at
http://www.ibiblio.org/webster/.
