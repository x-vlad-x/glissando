# Privacy

glissando is private by construction:

* **No network code.** No HTTP/TLS/DNS crates; the `glissando`,
  `glissando-settings` and `glissando-voice` binaries import no name
  resolution functions. When the keyboard runs from its systemd unit or as a
  trial, systemd forbids every socket family except `AF_UNIX`/`AF_NETLINK`
  (`RestrictAddressFamilies`), so even a bug could not reach the network.
  The settings app is a GTK application and does not run under this rule.
* **No telemetry, analytics, crash upload, accounts, remote flags or
  identifiers.**
* **No logging of typed text.** Both debug switches are off by default. A
  non-empty `GLISSANDO_DEBUG` logs state changes only: activate, surface size,
  frame times, the id of the focused app, navigation keys, modifier chords
  without their key, and the byte length of the word being composed.
  `GLISSANDO_DEBUG_INPUT`, when set to anything, also logs touch coordinates and
  key numbers. Typed text can be worked out from those, so use it only in test
  sessions. It logs nothing in sensitive fields, and no packaged unit or script
  sets it. A crash reports only the source location.
  `WAYLAND_DEBUG=1` is not ours: the Wayland library then prints every
  protocol message, including committed text, so never use it with real text.
* **Voice typing stays on the device.** Audio is held in memory and, during
  transcription, in a private `O_TMPFILE` file in `$XDG_RUNTIME_DIR` (the
  temporary directory if that is unset). The file never has a name, so it dies
  with the helper, even if the helper is killed. The speech engine is killed
  with the helper, and the keyboard kills the helper when the focus changes
  during transcription. The microphone is not offered in sensitive fields.
* **Sensitive fields** (content purpose password/PIN, hints
  `sensitive_data`/`hidden_text`): no composing text, no suggestions, no
  auto-correction, no learning, no glide typing, no microphone.
* **Personal data stays local and is erasable:**

  | Data | Location | Erase |
  |---|---|---|
  | learned words, personal dictionary, word pairs | `~/.local/share/glissando/user/*.tsv` (0600) | settings → Privacy & dictionary; quick menu → "Delete learned words" (languages of the enabled layouts only) |
  | recent emoji | `~/.local/state/glissando/recent-emoji` (0600) | settings → Privacy & dictionary |
  | clipboard history | memory only, expires after the configured hours | clipboard panel → Clear; turning history off; settings → Privacy & dictionary, while the keyboard runs |
  | pinned clipboard items | `~/.local/state/glissando/pinned-clips` (0600) | unpin; settings → Privacy & dictionary |

  "Delete learned words and data" in the settings removes the whole `user`
  directory, both state files and any `*.tmp` file in the state directory. A
  crash during a write can leave such a temporary copy. The running keyboard
  then drops its copies in memory, including the clipboard history. The
  keyboard also removes stale `*.tmp` files at each start. Personalization and
  "Remember recent emoji" stop new data being stored, but do not delete existing
  data.

  Deleting a file by hand works while the keyboard is stopped. While it runs, it
  keeps its copy in memory and can write the file again.

  `~/.local/state/glissando/crashes` holds only the times of recent failures,
  for the fallback to the system keyboard.

  Clipboard entries offered with a password-manager hint MIME type are
  never recorded. Words containing digits or `@` are never learned.
* **Licences and provenance** of bundled data: `data/dict/SOURCES.md`
  (AOSP word lists, Apache-2.0; Leipzig corpora, CC BY 4.0) and
  `data/emoji/LICENSE-UNICODE` (Unicode emoji/CLDR data).

## Verification

`tests/privacy/check.sh` runs in CI, in the e2e job, on every push to `main`
and every pull request:

1. dependency graph contains no networking crates,
2. `glissando`, `glissando-settings` and `glissando-voice` import no
   `getaddrinfo`/`gethostbyname`/…,
3. the complete end-to-end suite passes with an `LD_PRELOAD` socket guard
   on the keyboard that refuses and records any non-local socket or DNS
   lookup — the record must be empty,
4. the end-to-end suite passes inside an empty network namespace.

The script fails if the namespace cannot be created. The end-to-end suite does
not start the settings app, so steps 3 and 4 say nothing about it. It also fails
if a typed word appears in the debug log. CI has no speech model, so the
`voice_input` test reports `SKIP` there and steps 3 and 4 do not cover voice
typing in CI. They do when the model and test recording are present.

`cargo test` runs `crates/glissando-voice/tests/no_audio_left.rs`, which checks
with a simulated microphone and a scripted engine that no recording remains.

On the device: `systemd-run --user -p RestrictAddressFamilies="AF_UNIX AF_NETLINK"`
was verified to block `AF_INET` while glissando runs normally.
