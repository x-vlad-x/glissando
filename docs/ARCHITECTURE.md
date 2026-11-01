# Architecture

```
crates/
  glissando-config   config.toml, data directories, atomic private writes   (no deps but serde/toml)
  glissando-core     layouts (TOML), geometry/hit testing, touch state machine, text utils   (no deps but serde/toml)
  glissando-lang     mmap trie dictionaries, spatial fuzzy search, ranking, personal model,
                     language detection, typing engine (composing, auto-correct, smart punctuation)
  glissando-swipe    glide decoder (trie-pruned alignment + shape/location matching)
  glissando          the keyboard: Wayland client, cairo/pango renderer, D-Bus, haptics,
                     panels (emoji, clipboard, quick menu)
  glissando-settings GTK4/libadwaita settings app
  glissando-voice    speech helper: records audio and runs whisper-cli
  glissando-testkit  text-input-v3 test client and virtual pointer driver for e2e tests
data/                layouts, emoji (Unicode/CLDR), dictionaries (built by tools/build-dicts.py)
packaging/           cross build container, Debian packages, voice builds, device scripts (trial, session, deploy)
tests/               e2e (headless sway) and privacy checks
```

## Why an independent keyboard

* **Squeekboard** (GTK3, Rust+C): mature protocol handling but no
  prediction, a drawing model built around static YAML layouts and a GTK4
  port still pending. Reaching Gboard-level interaction would mean
  rewriting most of it.
* **Stevia** (GTK4, C/GObject): completion via presage/hunspell, emoji,
  cursor gestures; no glide typing. Its widget tree and GObject plumbing make
  per-frame animation (trails, popups) and multi-language ranking awkward.
* **Own client**: the platform surface is small and well defined
  (layer-shell + input-method-v2 + virtual-keyboard-v1 + `sm.puri.OSK0`),
  so we implement it directly with `wayland-client` and draw with cairo/pango
  (already on every Phosh system, handles fonts, colour emoji and
  fractional scaling). All keyboard logic is plain Rust, unit testable and
  shared with the e2e harness.

## Runtime structure

One thread, one glib main loop: the Wayland fd, timers (long press,
repeat, animations), D-Bus (`sm.puri.OSK0`, feedbackd) and the config file
monitor all dispatch on it. Rendering caches the static key layer and only
paints dynamic parts (pressed keys, previews, popups, trail, strip) per
frame; frames are paced by `wl_surface.frame`.

Input flows: `wl_touch`/`wl_pointer` → `Keyboard` (keyboard.rs) →
`TouchMachine` (taps, rollover, long press, repeat, glide detection,
space/backspace scrubbing) → `Engine` (glissando-lang) → `Op`s
(commit/preedit/delete/key) → `zwp_input_method_v2` batches or
`zwp_virtual_keyboard_v1` keys. Without an input-method client (shown
manually) text is typed through a dynamically generated XKB keymap.

## Configuration and personal data

`glissando-config` is shared by the keyboard and the settings app. Both write
`config.toml`, so neither may save a stale copy over the other.

* **`write_private`** writes to a uniquely named `.tmp` file next to the target
  with mode 0600, syncs it and renames it over the target. A crash leaves the
  old or the new file, never a partial one. It is used for `config.toml`, the
  learned word files, `recent-emoji` and `pinned-clips`.
* **Stale temporary files** can hold copies of personal data. The keyboard
  removes `*.tmp` from the config, user data and state directories at start,
  and the settings app removes them from the state directory when it deletes
  personal data. Deleting personal data removes the whole `user/` directory.
* **Config lock:** writes take an exclusive lock on `config.lock` next to the
  file, and read the file again under it. The settings app changes one value
  with `Config::update`. The keyboard saves quick menu changes with
  `Config::save_changes`, which writes only the keys that differ from its last
  saved copy.
* **Broken files:** both write paths stop if the file cannot be parsed, so a
  damaged file stays as it is. The keyboard keeps its copy and runs the save
  again the next time the file changes. At start, an invalid file means the
  defaults are used.
* **Data epochs:** `user_data_epoch` and `user_dict_epoch` are counters in
  `config.toml`. The settings app raises the first after deleting personal data
  and the second after editing the personal dictionary. The keyboard watches
  the file and keeps an `EpochTracker`, which treats only a value above the
  highest one seen as a request. A blank, partial or restored file can
  therefore never delete anything. On a request the keyboard drops its copies
  of learned words, recent emoji and clipboard history, or reloads the personal
  dictionary.

## Settings app

`glissando-settings` is a separate GTK4/libadwaita process. It never talks to
the keyboard directly: every switch goes through `Config::update`, and the
keyboard's file monitor picks up the change. It lists layouts from all resource
directories, edits the personal dictionary files, and deletes personal data.
Its strings are in `strings.rs`, in English, German and Russian.

## Voice typing

`glissando-voice` is a separate process in an optional package. The keyboard
starts it and exchanges lines over stdin and stdout: `listening`, `level`,
`processing`, `text` and `error` come back; `stop` and `cancel` go in. EOF on
stdin cancels, since nobody could receive the text.

The helper records 16 kHz mono audio with `parecord`, `pw-record` or `arecord`
into memory and stops on silence, on request or after 30 s. For transcription it
writes a WAV file with `O_TMPFILE` in `$XDG_RUNTIME_DIR`, so the file never has
a name, and passes `/proc/<pid>/fd/<n>` to `whisper-cli`. The engine runs with
`PR_SET_PDEATHSIG`, so it dies with the helper. The keyboard kills the helper
when the focus changes during transcription, and the kernel then frees the
audio. `crates/glissando-voice/tests/no_audio_left.rs` checks this with a
simulated microphone and a scripted engine.

## Language model

* Dictionary: character trie, BFS laid out, 16 bytes/node, memory mapped
  (`.gld`, see `dict.rs`), word frequencies on the AOSP 0–255 log scale,
  subtree max frequency for best-first completion, bigram table sorted by
  predecessor.
* Correction: bounded weighted edit distance over the trie (substitution,
  omission, insertion, transposition, doubled letters, diacritics) with
  per-tap key probabilities from the touch position (Gaussian spatial
  model).
* Ranking: −cost + log frequency + personal boost + bigram context +
  language prior. Multilingual typing: all enabled languages of the same
  script are active at once; an online detector shifts priors towards the
  language being written.
* Personalisation: per-language TSV in `$XDG_DATA_HOME/glissando/user/`
  (0600, written by `write_private`): learned words, personal dictionary (pinned), blocked words, word
  pair counts. Never fed from password/PIN/sensitive fields.

## Glide typing

`glissando-swipe::ShapeDecoder`: resamples the trace (48 points, key-width
units); walks the dictionary trie keeping only prefixes whose letters the
finger passed in order (pass radius 0.95 kw) starting/ending near the
gesture ends; scores survivors by a monotonic letter-to-path alignment,
location and shape distances to the ideal key polyline and a length
penalty; combines with frequency, personal and context scores. Layout and
language agnostic (works for ЙЦУКЕН, QWERTZ, letters in popups such as ё/ъ/ß
fold onto their base key).

Evaluation on **real human gestures** (FUTO swipe dataset, MIT licence,
held-out `test.jsonl`, 3000 in-vocabulary English words, QWERTY,
`cargo run --release -p glissando-swipe --example eval_futo`): top-1
87.7 %, top-3 95.1 % without context; 88.4 % / 95.4 % with the previous word
as context. Parameters were tuned on the `dev.jsonl` split only (from
75.6 % / 86.9 % before tuning).

Synthetic gestures (`--example bench`, 0.25 kw noise): en 98.0 % / 100 %,
de 96.3 % / 100 %, ru 97.0 % / 98.0 %. Latency: three-stage pruning (O(L)
bound → alignment for 700 → full score for 120) gives p95 ≈ 1.5 ms on
x86-64; on the phone p50 2.5 ms / p95 10 ms on a big core before the last
optimisation step.

FUTO Swipe backend (deferred): the published FUTO models exist only as
ExecuTorch `.pte` files. Running them needs the ExecuTorch runtime, whose
build requires a PyTorch toolchain for operator code generation and only
targets Android/desktop out of the box; the weights also carry their own
licence (visible "powered by FUTO Swipe" notice, not sublicensable). The
plan is a separate optional package that provides candidates to the
keyboard the same way `glissando-voice` provides transcripts, encoder-only
so that it works with our en/de/ru tries and layouts.
