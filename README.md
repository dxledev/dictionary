# Dictionary

A small, portable dictionary for Linux. English works offline from an embedded
Webster's 1913; 23 Wiktionary editions work online. Colors are set from the
command line, saved to one JSON file, and an open window reloads them the
moment the file changes. It is a native Rust window with no web runtime.

## Features

- **Words of the day.** The home page shows two distinct English featured words per source
  from Wiktionary and Urban Dictionary, using the same word layout as query
  results, with full part-of-speech labels, available pronunciation below the word,
  just the first definition as a preview and independent loading
  and retry controls.
  Select a word to look it up. Source dates follow the published feed; a feed
  may lag behind the current day. **Ctrl+U** also hides Urban on the home page.
  **Ctrl+1** and **Ctrl+2** open the first Wiktionary and Urban selections. Returning
  home refreshes selections cached for more than an hour. Selections persist in
  `$XDG_CACHE_HOME/dictionary/daily` (default `~/.cache/dictionary/daily`) and
  appear immediately on subsequent launches. Fresh caches skip network requests;
  older selections remain visible during background refresh and on network failure.
  Selections come from [Wiktionary's Word of the Day](https://en.wiktionary.org/wiki/Wiktionary:Word_of_the_day)
  and Urban's `words_of_the_day` feed.
- **Clear search.** The Clear button beside Search empties the field and returns home.
- **Offline English.** 108,181 headwords from Webster's 1913, embedded in the
  binary and decompressed one letter at a time, so a lookup never holds a
  whole letter in memory.
- **23 Wiktionary editions** (Arabic to Vietnamese), chosen in the in-window
  language panel or with `--lang`. English falls back to Wiktionary when a
  word is missing from the offline text.
- **Urban Dictionary.** English lookups also show up to five Urban Dictionary
  definitions in their own section, fetched in parallel so they never delay
  the dictionary result. Results share the dictionary layout, with author and
  date beneath each Urban definition. Word types are inferred from grammatical
  cues, with an optional NLTK fallback using the word in its example sentence;
  ambiguous descriptions can still remain unknown. It also fills in
  slang that Webster's and Wiktionary
  lack. Entries can be explicit; **Ctrl+U** hides the
  section. This uses Urban Dictionary's unofficial public JSON endpoint, whose
  terms say API access needs their permission, so check that before
  redistributing the app.
- **Independent loading.** Each source shows its results as soon as they arrive,
  with one shared loading panel while both are pending and a separate loading
  panel for the remaining source once either finishes. Empty sources stay
  hidden; if neither finds a definition, one final empty state appears.
- **Heuristic non-English parsing.** English Wiktionary entries come from its
  structured definitions API. Other editions are read from page text, whose
  layout differs per edition, so senses are best-effort: French, German,
  Spanish, Italian, Turkish and Japanese read well; some editions (for
  example Russian and Polish) may show extra sections or nothing.
- **Explore similar words.** English results end with up to 12 clickable word
  badges, with no source labels or scores. Selecting a badge replaces the search
  query and looks it up. Recommendations compare the current dictionary and
  enabled Urban definitions with the embedded English dictionary, using
  definition overlap and word references, with smaller bonuses for shared word
  types, identical pronunciation and word prefixes. This is a lexical heuristic;
  the bundled data has no dedicated etymology field. Recommendations run locally
  in the background and cancel on query changes. Each search randomly samples
  a broader pool of related words, favoring stronger matches while allowing
  looser matches to appear too. Badges remain stable until the next search.
  Pointer hover lifts badges by 3px over 120ms; reduced motion keeps them still.
  No extra network requests or saved query history are needed. The section is
  hidden when no relevant matches exist and in non-English editions. Set
  `DICTIONARY_SIMILAR_LIMIT=0` to disable it, or choose a limit from 1 to 24
  (default 12). `DICTIONARY_SIMILAR_POOL` controls the candidate pool size
  (default 96, at least the badge limit and at most 192).
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

## Urban Dictionary word types

Existing definition rules run first. For unresolved entries, an optional Python
helper tags the actual headword in its example sentence in the background.
It never tags definition prose to guess the headword's type. Explicitly mixed
word types, conflicting uses, missing examples and multiword expressions can
still show `N/A`. NLTK is trained on conventional English, so slang labels
remain estimates.

Definitions beginning with noun phrases such as “people who are…”, “someone
who thinks…” or “a device that works…” are treated as nouns. Explicit word
types and existing verb cues take precedence.

Definitions with an inferred word type appear before `N/A` definitions. The
original result order is preserved within each set.

Install the optional environment and English model once (requires Python 3.10+
and internet access):

```sh
bash scripts/setup-urban-pos.sh --dry-run
bash scripts/setup-urban-pos.sh
```

The default environment is `$XDG_DATA_HOME/dictionary/nltk`, or
`~/.local/share/dictionary/nltk`. Use `--prefix /absolute/path` to install
elsewhere, then set `DICTIONARY_NLTK_PYTHON=/absolute/path/bin/python` and
`NLTK_DATA=/absolute/path/nltk_data` when launching Dictionary. The helper is
embedded in the Rust binary; the Python environment and model remain separate.
There are no downloads during lookups. Each helper process has a three-second
timeout. Missing dependencies or a helper failure leave the normal dictionary
results available. Setup installs NumPy for NLTK's confidence-score calculation.

Configuration:

- `DICTIONARY_NLTK=0` disables the fallback.
- `DICTIONARY_NLTK_PYTHON` selects a Python interpreter. Otherwise the managed
  environment above is preferred, with `python3` as the final interpreter choice.
- `NLTK_DATA` selects installed model data.
- `DICTIONARY_NLTK_MIN_CONFIDENCE` sets the model-score cutoff (0–1, default 0.9).
  This score is not a calibrated probability of slang correctness.

The helper uses NLTK's English `averaged_perceptron_tagger_eng` JSON model and
Treebank tokenization; no `punkt` download is needed. See the
[NLTK tagger documentation](https://www.nltk.org/api/nltk.tag.PerceptronTagger.html).

Run the existing isolated GPUI harness and fallback tests without opening a
window on the desktop:

```sh
cargo test --offline
python3 -m unittest discover -s scripts -p 'test_urban_pos.py'
# Optional check after installing NLTK and its model:
cargo test --offline installed_model_classifies_headword_in_context -- --ignored
```

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

Motion: panels fade and slide into place over 120 ms (ease-out-cubic). Set
`DICTIONARY_REDUCED_MOTION=1` to make them instant. The search icon pulses while
a source is loading, and loading panels show a rotating spinner. Reduced motion
keeps both indicators static.

## Data and licenses

Code: MIT. The embedded Webster's 1913 data is derived from GCIDE and is
GPL-3.0-or-later (the 1913 text itself is public domain); see
`data/webster/LICENSE-DATA.txt`. Wiktionary content is available under
CC BY-SA. Urban Dictionary text belongs to its authors and is fetched at
lookup time, never stored. The common-word list used for suggestions is derived from Google's
10,000 most common English words.

The offline data was built from the GCIDE XML 0.53 release published at
http://www.ibiblio.org/webster/.
