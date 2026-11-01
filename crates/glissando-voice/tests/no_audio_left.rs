//! Runs the real helper with a simulated microphone and a scripted speech
//! engine, and checks that no recording remains on any way out.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Run {
    root: PathBuf,
    runtime: PathBuf,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gv-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("runtime")).unwrap();
    d
}

fn engine_script(root: &Path, body: &str) -> PathBuf {
    let path = root.join("engine");
    let script = format!(
        "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -f ] && audio=$2; shift; done\nroot={}\n{body}\n",
        root.display()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn start(root: PathBuf, engine: &Path) -> Run {
    let runtime = root.join("runtime");
    let mic = root.join("mic.raw");
    let loud: Vec<u8> =
        (0..16_000).flat_map(|i| (if i % 2 == 0 { 20_000i16 } else { -20_000 }).to_le_bytes()).collect();
    std::fs::write(&mic, loud).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_glissando-voice"))
        .args(["--model", "unused", "--engine"])
        .arg(engine)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("GLISSANDO_VOICE_TEST_INPUT", &mic)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    Run { root, runtime, child, stdin, stdout }
}

impl Run {
    fn read_line(&mut self) -> String {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    }

    /// Lets the helper hear speech, then sends `command`.
    fn speak_then(&mut self, command: &str) {
        let mut levels = 0;
        while levels < 5 {
            let line = self.read_line();
            assert!(!line.is_empty() && !line.starts_with("error"), "{line:?}");
            levels += line.starts_with("level ") as u32;
        }
        writeln!(self.stdin, "{command}").unwrap();
    }

    /// The first line with `prefix`, or an empty string at end of output.
    fn line_starting(&mut self, prefix: &str) -> String {
        loop {
            let line = self.read_line();
            if line.starts_with(prefix) || line.is_empty() {
                return line;
            }
        }
    }

    fn assert_no_audio_left(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let left: Vec<_> = std::fs::read_dir(&self.runtime).unwrap().flatten().map(|e| e.path()).collect();
        assert!(left.is_empty(), "{left:?}");
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn engine_reads_the_audio_of_an_unnamed_file() {
    let root = temp_dir("success");
    let engine = engine_script(
        &root,
        "wc -c < \"$audio\" > \"$root/size\"\nls -A \"$root/runtime\" > \"$root/listing\"\necho hello",
    );
    let mut run = start(root.clone(), &engine);
    run.speak_then("stop");
    assert_eq!(run.line_starting("text"), "text hello");
    let size: usize = std::fs::read_to_string(root.join("size")).unwrap().trim().parse().unwrap();
    assert!(size > 44, "{size}");
    assert_eq!(std::fs::read_to_string(root.join("listing")).unwrap(), "");
    run.assert_no_audio_left();
}

#[test]
fn failing_engine_leaves_nothing() {
    let root = temp_dir("failure");
    let engine = engine_script(&root, "echo broken >&2\nexit 3");
    let mut run = start(root, &engine);
    run.speak_then("stop");
    assert!(run.line_starting("error").contains("broken"));
    run.assert_no_audio_left();
}

#[test]
fn missing_engine_leaves_nothing() {
    let root = temp_dir("missing");
    let engine = root.join("no-such-engine");
    let mut run = start(root, &engine);
    run.speak_then("stop");
    assert!(run.line_starting("error").starts_with("error"));
    run.assert_no_audio_left();
}

#[test]
fn cancelling_leaves_nothing() {
    let root = temp_dir("cancel");
    let marker = root.join("engine-ran");
    let engine = engine_script(&root, "touch \"$root/engine-ran\"");
    let mut run = start(root, &engine);
    run.speak_then("cancel");
    assert_eq!(run.line_starting("text"), "");
    assert!(!marker.exists(), "cancelling must not transcribe");
    run.assert_no_audio_left();
}

#[test]
fn killing_the_helper_during_transcription_leaves_nothing() {
    let root = temp_dir("killed");
    let started = root.join("started");
    let engine = engine_script(
        &root,
        "echo $$ > \"$root/started.tmp\"\nmv \"$root/started.tmp\" \"$root/started\"\nwhile :; do :; done",
    );
    let mut run = start(root, &engine);
    run.speak_then("stop");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the engine did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = std::fs::read_to_string(&started).unwrap();
    let cmdline = format!("/proc/{}/cmdline", pid.trim());
    run.assert_no_audio_left();
    // A killed process that nobody has reaped yet has an empty command line.
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read(&cmdline).is_ok_and(|c| !c.is_empty()) {
        assert!(Instant::now() < deadline, "the engine outlived the helper");
        std::thread::sleep(Duration::from_millis(10));
    }
}
