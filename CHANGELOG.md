# Changelog

## Unreleased

- The keyboard registers with gnome-session, which Phosh requires of a
  required session component. Without it, sessions started from
  `sm.puri.OSK0.desktop` failed with "Oh no! Something has gone wrong".
  Thanks to [@VipinVIP](https://github.com/VipinVIP) for tracking this down
  on a system without systemd and proposing the fix (#1)

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
