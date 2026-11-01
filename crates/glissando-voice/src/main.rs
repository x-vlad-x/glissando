//! glissando-voice: local speech-to-text helper for the keyboard.
//!
//! Records from the default audio source with a system tool (parecord,
//! pw-record or arecord), stops on silence or on request, and transcribes
//! with a local whisper.cpp model. Audio only ever lives in memory and in an
//! unnamed private file in `$XDG_RUNTIME_DIR` (tmpfs), so no failure path and
//! no signal can leave a recording behind.
//!
//! Protocol (one line per message on stdout):
//!   `listening` · `level <0..1>` · `processing` · `text <utf8>` · `error <msg>`
//! stdin: `stop` (or EOF) ends recording and transcribes; `cancel` aborts.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 16_000;

fn say(msg: &str) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{msg}");
    let _ = o.flush();
}

fn recorders() -> Vec<Vec<String>> {
    let r = RATE.to_string();
    vec![
        ["parecord", "--raw", "--format=s16le", &format!("--rate={r}"), "--channels=1", "--latency-msec=40"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ["pw-record", "--format", "s16", "--rate", &r, "--channels", "1", "-"].iter().map(|s| s.to_string()).collect(),
        ["arecord", "-q", "-f", "S16_LE", "-r", &r, "-c", "1", "-t", "raw"].iter().map(|s| s.to_string()).collect(),
    ]
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        for p in [dir.join(name), dir.join("../libexec/glissando").join(name)] {
            if p.exists() {
                return Some(p);
            }
        }
    }
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(name)).find(|p| p.exists())
}

/// Model search: --model, then `voice/` in the glissando data directories.
pub fn find_model(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("GLISSANDO_DATA_DIR") {
        dirs.push(PathBuf::from(d));
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    dirs.push(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("glissando"),
    );
    for d in std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into()).split(':') {
        dirs.push(Path::new(d).join("glissando"));
    }
    for d in dirs {
        let v = d.join("voice");
        if !name.is_empty() && v.join(name).is_file() {
            return Some(v.join(name));
        }
        if let Ok(rd) = std::fs::read_dir(&v) {
            let mut models: Vec<PathBuf> =
                rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "bin").unwrap_or(false)).collect();
            models.sort();
            if let Some(m) = models.into_iter().next() {
                return Some(m);
            }
        }
    }
    None
}

/// Stores the recording in a file that loses its name before any audio is
/// written. The keyboard kills this helper when focus changes during
/// transcription, so removing a named file afterwards would not be enough.
fn spool_wav(dir: &Path, samples: &[i16]) -> std::io::Result<std::fs::File> {
    let path = dir.join(format!("glissando-voice-{}.wav", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true).create_new(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let file = opts.open(&path)?;
    std::fs::remove_file(&path)?;
    write_wav(&file, samples)?;
    Ok(file)
}

/// The path through which another process of this user can open `file`
/// while this process is alive.
fn proc_path(file: &std::fs::File) -> PathBuf {
    let fd = std::os::fd::AsRawFd::as_raw_fd(file);
    PathBuf::from(format!("/proc/{}/fd/{fd}", std::process::id()))
}

fn write_wav(out: impl Write, samples: &[i16]) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(out);
    let data = (samples.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&RATE.to_le_bytes())?;
    f.write_all(&(RATE * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// Phrases Whisper is known to invent on silence or noise (subtitle credits
/// from its training data). Sentences containing them are dropped.
const HALLUCINATIONS: &[&str] = &[
    "редактор субтитров",
    "корректор а.",
    "субтитры сделал",
    "субтитры создавал",
    "продолжение следует",
    "спасибо за просмотр",
    "подписывайтесь на канал",
    "thank you for watching",
    "thanks for watching",
    "subtitles by",
    "please subscribe",
    "untertitel im auftrag",
    "untertitel von",
    "untertitelung",
    "vielen dank für's zuschauen",
    "amara.org",
];

fn drop_hallucinations(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    for (i, c) in &bytes {
        if matches!(c, '.' | '!' | '?' | '…') {
            out.push(&text[start..i + c.len_utf8()]);
            start = i + c.len_utf8();
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    // Credits like "А.Синецкая" contain dots: judge by the whole text first.
    let lower = text.to_lowercase();
    if HALLUCINATIONS.iter().any(|h| lower.contains(h))
        && HALLUCINATIONS.iter().any(|h| lower.trim_start().starts_with(h))
    {
        return String::new();
    }
    out.into_iter()
        .filter(|s| {
            let l = s.to_lowercase();
            !HALLUCINATIONS.iter().any(|h| l.contains(h))
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// Remove whisper.cpp annotations such as "[BLANK_AUDIO]" or "(music)".
pub fn clean_transcript(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in raw.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    drop_hallucinations(&out.split_whitespace().collect::<Vec<_>>().join(" "))
}

struct Args {
    lang: String,
    model: Option<PathBuf>,
    engine: Option<PathBuf>,
    max_seconds: u64,
    threads: usize,
    /// Test mode: read 16 kHz mono s16le samples from this file instead of a microphone.
    input: Option<PathBuf>,
}

fn parse_args() -> Args {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let input = std::env::var_os("GLISSANDO_VOICE_TEST_INPUT").map(PathBuf::from);
    let mut a = Args { lang: "auto".into(), model: None, engine: None, max_seconds: 30, threads, input };
    let v: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < v.len() {
        let next = v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--lang" => a.lang = next,
            "--model" => a.model = Some(next.into()),
            "--engine" => a.engine = Some(next.into()),
            "--max-seconds" => a.max_seconds = next.parse().unwrap_or(30),
            "--threads" => a.threads = next.parse().unwrap_or(4),
            "--input" => a.input = Some(next.into()),
            "--check" => {
                // Exit 0 when a model and an engine are installed.
                let ok = find_model("").is_some() && find_in_path("whisper-cli").is_some();
                std::process::exit(if ok { 0 } else { 1 });
            }
            _ => {}
        }
        i += 2;
    }
    a
}

fn main() {
    let args = parse_args();
    let Some(model) = args.model.clone().or_else(|| find_model("")) else {
        say("error no speech model installed");
        std::process::exit(1);
    };
    let Some(engine) = args.engine.clone().or_else(|| find_in_path("whisper-cli")) else {
        say("error whisper-cli not found");
        std::process::exit(1);
    };
    // Control messages from the keyboard.
    let (ctl_tx, ctl_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    let _ = ctl_tx.send(l.trim().to_string());
                }
                Err(_) => break,
            }
        }
        let _ = ctl_tx.send("stop".into());
    });
    // Start a recorder.
    let samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(AtomicBool::new(false));
    if let Some(f) = &args.input {
        // Simulated microphone for tests: real-time playback of a raw file.
        let data = std::fs::read(f).unwrap_or_default();
        let s2 = samples.clone();
        let d2 = done.clone();
        std::thread::spawn(move || {
            for chunk in data.chunks(3200) {
                if d2.load(Ordering::Relaxed) {
                    return;
                }
                s2.lock()
                    .unwrap()
                    .extend(chunk.chunks(2).filter(|c| c.len() == 2).map(|c| i16::from_le_bytes([c[0], c[1]])));
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }
    let mut child = None;
    let recs = if args.input.is_some() { Vec::new() } else { recorders() };
    for cmd in recs {
        if let Ok(c) = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            child = Some(c);
            break;
        }
    }
    if child.is_none() && args.input.is_none() {
        say("error no audio recorder (parecord, pw-record or arecord)");
        std::process::exit(1);
    }
    let mut rec = child;
    if let Some(r) = rec.as_mut() {
        let mut out = r.stdout.take().unwrap();
        let s2 = samples.clone();
        let d2 = done.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 3200];
            let mut carry: Option<u8> = None;
            while !d2.load(Ordering::Relaxed) {
                match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut bytes: Vec<u8> = carry.take().into_iter().collect();
                        bytes.extend_from_slice(&buf[..n]);
                        if bytes.len() % 2 == 1 {
                            carry = bytes.pop();
                        }
                        let mut s = s2.lock().unwrap();
                        s.extend(bytes.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])));
                    }
                }
            }
        });
    }
    say("listening");
    let start = Instant::now();
    let mut last_level = 0usize;
    let mut speech = false;
    let mut silent_since: Option<Instant> = None;
    let mut cancelled = false;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        match ctl_rx.try_recv() {
            Ok(c) if c == "cancel" => {
                cancelled = true;
                break;
            }
            Ok(_) => break,
            Err(_) => {}
        }
        let (rms, n) = {
            let s = samples.lock().unwrap();
            let win = &s[last_level.min(s.len())..];
            let rms = if win.is_empty() {
                0.0
            } else {
                (win.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / win.len() as f64).sqrt() / 32768.0
            };
            (rms, s.len())
        };
        last_level = n;
        let level = (rms * 8.0).min(1.0);
        say(&format!("level {level:.2}"));
        if level > 0.12 && start.elapsed() > Duration::from_millis(300) {
            speech = true;
            silent_since = None;
        } else if speech {
            let since = *silent_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_millis(1600) {
                break;
            }
        } else if start.elapsed() > Duration::from_secs(8) {
            break;
        }
        if start.elapsed() > Duration::from_secs(args.max_seconds) {
            break;
        }
    }
    done.store(true, Ordering::Relaxed);
    if let Some(mut r) = rec {
        let _ = r.kill();
        let _ = r.wait();
    }
    // The reader thread is not joined: a stuck recorder must not block us.
    if cancelled {
        return;
    }
    let audio = std::mem::take(&mut *samples.lock().unwrap());
    if !speech || audio.len() < (RATE / 4) as usize {
        say("text ");
        return;
    }
    say("processing");
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let wav = match spool_wav(&dir, &audio) {
        Ok(f) => f,
        Err(e) => {
            say(&format!("error {e}"));
            return;
        }
    };
    drop(audio);
    let threads = args.threads.max(1).to_string();
    let result = Command::new(&engine)
        .args(["-m"])
        .arg(&model)
        .args(["-f"])
        .arg(proc_path(&wav))
        // Greedy decoding: ~25 % faster than beam search on phone CPUs.
        .args(["-l", &args.lang, "-nt", "-np", "-sns", "-bs", "1", "-bo", "1", "-t", &threads])
        .stdin(Stdio::null())
        .output();
    drop(wav);
    match result {
        Ok(o) if o.status.success() => say(&format!("text {}", clean_transcript(&String::from_utf8_lossy(&o.stdout)))),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
            say(&format!("error speech engine failed ({}): {last}", o.status))
        }
        Err(e) => say(&format!("error {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_cleanup() {
        assert_eq!(clean_transcript(" [BLANK_AUDIO]\n Привет,  мир! (music)\n"), "Привет, мир!");
        assert_eq!(clean_transcript(" Редактор субтитров А.Синецкая Корректор А.Егорова"), "");
        assert_eq!(clean_transcript("Hello there. Thanks for watching!"), "Hello there.");
    }

    #[test]
    fn finds_model_file_not_directory() {
        let d = std::env::temp_dir().join(format!("gv-models-{}", std::process::id()));
        std::fs::create_dir_all(d.join("voice")).unwrap();
        std::fs::write(d.join("voice/ggml-base-q8_0.bin"), b"x").unwrap();
        // SAFETY: test-only, single threaded at this point.
        unsafe { std::env::set_var("GLISSANDO_DATA_DIR", &d) };
        assert_eq!(find_model("").unwrap(), d.join("voice/ggml-base-q8_0.bin"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn wav_header() {
        let d = std::env::temp_dir().join(format!("gv-spool-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = spool_wav(&d, &[0, 1, -1]).unwrap();
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0, "the audio file must have no name");
        let b = std::fs::read(proc_path(&f)).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 6);
        std::fs::remove_dir_all(d).unwrap();
    }
}
