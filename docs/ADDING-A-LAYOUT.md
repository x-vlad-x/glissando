# Adding a layout

New languages and layouts beyond English, German and Russian are outside the
scope of 1.0 (see [RELEASE-1.0.md](RELEASE-1.0.md)). A new layout can still be
used locally, and proposals can return in a later 1.x release. That document also
decides how stable the layout format is.

A language needs up to three things: a layout file, a dictionary and emoji
keywords. Only the layout file is required.

## The layout file

Layouts are in `data/layouts/<id>.toml`. Take the closest of `en.toml`,
`de.toml` and `ru.toml` as a start.

```toml
id = "ru"              # must match the file name
name = "Русский"       # shown in the settings
short = "RU"           # label on the keyboard
languages = ["ru"]     # dictionaries used with this layout

[alternates]           # long-press popups
"е" = "ё"

[layers.letters]       # required
rows = [
  { keys = "й ц у к е н г ш щ з х", hints = "1 2 3 4 5 6 7 8 9 0 _" },
  { keys = "ф ы в а п р о л д ж э", hints = "@ # ₽ _ & - + ( ) _ _" },
  { keys = "{shift} я ч с м и т ь б ю {backspace}", hints = "* \" ' : ; ! ? _ _" },
]
bottom = "letters"
```

### Keys

Tokens in `keys` are separated by spaces.

| Token | Meaning |
|---|---|
| `q` | Types `q`; a label of several characters works too |
| `q:1.5` | The same key, 1.5 units wide |
| `{shift:1.5}` | A function key with a width |
| `{gap:0.5}` | Empty space |
| `{text=.com:1.5}` | A text key with characters the syntax reserves |
| `{layer=symbols:1.5}` | Switches to a layer |
| `{space:*}` | `*` fills the row up to the widest row of the layer |

Function keys are listed in `KeyAction::from_name` in
`crates/glissando-core/src/layout.rs`, for example `shift`, `backspace`, `enter`,
`space`, `lang`, `emoji`, `comma` and `period`.

Rows of a layer should have the same width in units. In `en.toml`, the second
row uses `{gap:0.5}` on both sides to reach 10 units.

### Hints and popups

`hints` gives one label per text key, in order; function keys are skipped. `_`
means no hint. A hint is the first entry of the key's long-press popup, followed
by its `[alternates]`. Alternates are separated by spaces. A value without spaces
is split into characters. Entries in the layout's `[alternates]` replace those
for the same key in `symbols.toml`.

### Shared layers

`data/layouts/symbols.toml` holds the symbol and number layers and the bottom
rows, and they are merged into every layout. A layout can replace a layer by
defining one with the same name. `bottom = "letters"` names a template from the
`[bottom]` table of that file; any other value is read as a row of keys. The
email and URL rows come from the `letters_email` and `letters_url` templates, so
a layer that uses `bottom = "letters"` gets them without further work.

## Trying it

Layouts are searched in `GLISSANDO_DATA_DIR`, then in
`~/.local/share/glissando`, then in the `glissando` directory of each
`XDG_DATA_DIRS` entry. Put the file in `~/.local/share/glissando/layouts/`, then
enable it in the settings under the keyboard layouts. If a file does not parse,
the keyboard writes the error to the journal and skips it.

## The dictionary

For each code in `languages`, the keyboard opens `dict/<code>.gld` from the same
directories. Without one it still works, with suggestions limited to learned
words. It logs `no dictionary for <code>`. Glide typing also walks the dictionary.

To build one, add the language to `LANGS` in `tools/build-dicts.py`, with the
AOSP word list name and the Leipzig corpus name, and add the SHA-256 of both
files to `SHA256`. Then run the script without arguments, since it rewrites
`data/dict/SOURCES.md` with the languages it built. Commit `SOURCES.md`, not the
`.gld` file. Any other word list needs a licence that allows distribution.

## Other places a language appears

* **Multilingual typing:** enabled layouts are treated as Latin or non-Latin. The
  languages of all layouts on the same side are active at once.
* **Letters in popups:** the glide decoder folds letters onto their base key.
  `ё`, `ъ` and `ß` are named in `crates/glissando-swipe/src/lib.rs`; others go
  through `fold_char` in `glissando-core`.
* **Emoji search:** `LANGS` in `tools/build-emoji.py`.
* **Interface language:** `crates/glissando/src/i18n.rs` and
  `crates/glissando-settings/src/strings.rs`. This is separate from layouts.
* **Packages:** `build-deb.sh` ships every file in `data/layouts` and every
  `.gld` file. The dictionary archive in `.github/workflows/release.yml` lists
  `en`, `de` and `ru` by name.

## Tests

`shipped_layouts_parse` in `crates/glissando-core/src/layout.rs` lists the shipped
layouts with their letter count and row width. Add a line for a new shipped
layout, then run `cargo test -p glissando-core`.
