# Changelog

## Unreleased

- Typing works in Flutter apps such as FluffyChat and Flexify. Flutter
  includes the word being typed in the text it reports back to the keyboard,
  which the keyboard took as the word already finished: every letter after
  the first then replaced the one before. The keyboard now replaces the
  reported word itself, which also covers phoc and Qt finishing the word on
  a touch. Thanks to [@nico359](https://github.com/nico359) for the report
  (#47)
- Glide delete over several words in Flutter apps removes the words behind
  the word being typed too, not only that word
- Voice typing no longer forces a language the typing has not clearly shown:
  with English and German enabled, English speech was recognised as German.
  The speech engine now detects the language itself until one is clearly
  ahead, and a Russian layout still selects Russian
- Text inserted after Enter (voice typing, emoji, clipboard) no longer starts
  with a space in apps that do not mark their text view as multiline, such
  as GNOME Text Editor
- German texts in the settings app and the quick menu are reviewed: glide
  typing is now "Wischeingabe", and several labels read more naturally (#25)
- postmarketOS is listed as not supported in 1.0: there is no APK package,
  and the keyboard is started by a systemd user unit (#19)
- Release files carry a signed build attestation that
  `gh attestation verify` checks (#16)
- The README describes the versioning and support policy from 1.0.0 on (#32)

## 1.0.0-rc.2 – second release candidate

Bug fixes for text editing found while testing 1.0.0-rc.1 on the FLX1s.

- Backspace and the backspace glide no longer garble text after you move the
  cursor or select text by touching the field. GTK does not tell the
  keyboard about these, so the keyboard deleted letters at the old cursor
  position ("Hello world foo" became "Hello wfo|d foo") or deleted only part
  of a selection. Until the app reports the text again, deletions now go out
  as key presses, which the app applies at its real cursor
- A word reopened with backspace is no longer auto-corrected when you finish
  it without typing new letters
- Touching the field in the middle of a sentence no longer turns on shift

## 1.0.0-rc.1 – first release candidate

From this candidate on, only bug fixes go into 1.0.0. The scope and the
compatibility promise are in [docs/RELEASE-1.0.md](docs/RELEASE-1.0.md).

- Packages are available from the signed APT repository at
  <https://deb.xvlad.com>, so updates arrive with `apt upgrade` (#21)
- The Debian version now carries a revision: this candidate is
  `1.0.0~rc1-1`, and 1.0.0 will be `1.0.0-1`, which sorts as newer
- The keyboard registers with gnome-session, which Phosh requires of a
  required session component. Without it, sessions started from
  `sm.puri.OSK0.desktop` failed with "Oh no! Something has gone wrong".
  Thanks to [@VipinVIP](https://github.com/VipinVIP) for tracking this down
  on a system without systemd and proposing the fix (#1)
- A recording no longer stays in the runtime directory when a field loses
  focus during transcription: the audio file now has no name (#23)
- `GLISSANDO_DEBUG` no longer logs the letter of Ctrl and Alt shortcuts,
  `GLISSANDO_DEBUG_INPUT` logs nothing in password fields, and crash messages
  no longer quote text (#23)
- With the automatic theme, a change of the system colour scheme applies at
  once (#5)
- Turning the screen keyboard off or on in the system settings applies at
  once instead of at the next login (#4)
- The settings app no longer overwrites changes made from the keyboard's
  quick menu while it is open (#3)
- Recent emoji and pinned clipboard items are written atomically and readable
  only by you, so a crash during a write can no longer lose them (#6)
- Glide typing is off in password and other sensitive fields, where its
  candidates showed the typed word in the suggestion strip
- "Delete learned words" in the quick menu also deletes the words of languages
  that are not enabled
- A damaged `config.toml` no longer deletes learned words and is no longer
  replaced by the defaults; quick menu changes are saved once it is fixed
- During fast typing the field no longer keeps showing an incomplete word:
  an update that the compositor dropped because it crossed a reply from the
  app is sent again
- After a crash, systemd restarts the keyboard again, and after three
  crashes within five minutes Stevia or Squeekboard takes over. Until now
  the keyboard service stopped after the first crash, leaving no screen
  keyboard (#9)
- `glissando-voice --help` prints usage instead of starting to record, and
  unknown options are rejected (#42)

## 0.3.0 – third preview

- The second letter of a word no longer replaces the first when typing into
  a field whose text is selected (GNOME Maps search after picking a result):
  a text change around the preedit keeps the word being composed
- Words are no longer doubled or split after touching the app while
  composing on Phosh: phoc commits the preedit itself, which is now
  recognised
- A word being composed when the field loses focus is no longer typed into
  whatever has the focus next
- Long press on the Phosh home bar shows and hides the keyboard, as with
  Stevia: visibility changes now reach Phosh (the `PropertiesChanged`
  signal had the wrong signature)

## 0.2.0 – second preview

- Words no longer split in two after moving the cursor in Qt apps and other
  clients that do not report why the text changed: edits are recognised by
  comparing the text, late reports of our own typing are ignored
- Backspace with a selection deletes the selection with a key press
- A field without surrounding text no longer inherits the previous field's
- Terminal keys: Esc, Tab, Ctrl, Alt and arrows above the letters in
  terminals (and when typing without an input-method client); Ctrl and Alt
  apply to the next key, arrows repeat while held
- Apps where the keyboard does not open by itself (Calculator by default),
  with a settings page to pick installed apps and import Stevia's list

## 0.1.0 – first preview

- Wayland keyboard for Phosh: input-method-v2, virtual keyboard fallback,
  `sm.puri.OSK0`, layer-shell, fractional scaling, portrait and landscape
- Layouts: English (QWERTY), German (QWERTZ), Russian (ЙЦУКЕН), symbols,
  number/phone/PIN pads, email and URL variants
- Glide typing with trail; space-bar cursor control; glide delete
- Suggestions, next-word prediction, spatial auto-correction with undo,
  multilingual typing with language detection, on-device learning
- Emoji panel with search and suggestions, clipboard history, quick menu,
  one-handed mode, number row
- Local voice typing (whisper.cpp) as an optional package
- GTK4/libadwaita settings app (EN/DE/RU)
- Debian packages registering a non-default Phosh-OSK alternative
- Privacy: no network code, kernel-enforced network ban, automated checks
