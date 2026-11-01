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
* **No logging of typed text.** `GLISSANDO_DEBUG=1` logs state changes only:
  activate, surface size, frame times, the id of the focused app, modifier
  chords without their key, and the byte length of the word being composed.
  `GLISSANDO_DEBUG_INPUT=1` logs touch coordinates and key numbers, from which
  typed text can be worked out, so it must only be used in test sessions; it
  logs nothing in sensitive fields. A crash reports only the source location.
  `WAYLAND_DEBUG=1` is not ours: the Wayland library then prints every
  protocol message, including committed text, so never use it with real text.
* **Voice typing stays on the device.** Audio is held in memory and, during
  transcription, in a private unnamed file in `$XDG_RUNTIME_DIR`. The file has
  no name to leave behind, even if the helper is killed.
* **Sensitive fields** (content purpose password/PIN, hints
  `sensitive_data`/`hidden_text`): no composing text, no suggestions, no
  auto-correction, no learning, no glide typing.
* **Personal data stays local and is erasable:**

  | Data | Location | Erase |
  |---|---|---|
  | learned words, personal dictionary, word pairs | `~/.local/share/glissando/user/*.tsv` (0600) | quick menu → "Delete learned words", or delete the files |
  | recent emoji | `~/.local/state/glissando/recent-emoji` | delete the file |
  | clipboard history | memory only, expires after the configured hours | strip → clipboard → Clear; turning history off |
  | pinned clipboard items | `~/.local/state/glissando/pinned-clips` (0600) | unpin, or delete the file |

  Clipboard entries offered with a password-manager hint MIME type are
  never recorded. Words containing digits or `@` are never learned.
* **Licences and provenance** of bundled data: `data/dict/SOURCES.md`
  (AOSP word lists, Apache-2.0; Leipzig corpora, CC BY 4.0) and
  `data/emoji/LICENSE-UNICODE` (Unicode emoji/CLDR data).

## Verification

`tests/privacy/check.sh` (run before releases; CI runs step 1 only):

1. dependency graph contains no networking crates,
2. `glissando`, `glissando-settings` and `glissando-voice` import no
   `getaddrinfo`/`gethostbyname`/…,
3. the complete end-to-end suite passes with an `LD_PRELOAD` socket guard
   on the keyboard that refuses and records any non-local socket or DNS
   lookup — the record must be empty,
4. the end-to-end suite passes inside an empty network namespace.

The end-to-end suite does not start the settings app, so steps 3 and 4 say
nothing about it. It also fails if a typed word appears in the debug log.

On the device: `systemd-run --user -p RestrictAddressFamilies="AF_UNIX AF_NETLINK"`
was verified to block `AF_INET` while glissando runs normally.
