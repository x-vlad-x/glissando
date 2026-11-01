//! Evaluate the glide decoder on real human gestures from the FUTO swipe
//! dataset (MIT, https://huggingface.co/datasets/futo-org/swipe.futo.org).
//! `cargo run --release -p glissando-swipe --example eval_futo -- test.jsonl qwerty.json [N]`

use glissando_lang::dict::Dict;
use glissando_lang::model::Language;
use glissando_lang::user::UserModel;
use glissando_swipe::{KeyMap, ShapeDecoder, clean_trace_timed};
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = &args[1];
    let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data");
    let dict = Dict::open(std::path::Path::new(&format!("{root}/dict/en.gld"))).unwrap();
    let langs = vec![Language::new("en", Some(dict), UserModel::in_memory())];
    let spec = std::env::var("PARAMS").unwrap_or_default();
    let dec = ShapeDecoder { params: glissando_swipe::DecodeParams::default().with_overrides(&spec) };
    let quiet = std::env::var_os("QUIET").is_some();
    let (mut total, mut top1, mut top3, mut oov) = (0, 0, 0, 0);
    let mut lat: Vec<f64> = Vec::new();
    let mut misses = Vec::new();
    let use_ctx = std::env::var_os("CONTEXT").is_some();
    let mut prev_word: Option<(String, String)> = None;
    for line in std::io::BufReader::new(std::fs::File::open(data).unwrap()).lines() {
        if total >= n {
            break;
        }
        let Ok(rec) = serde_json::from_str::<serde_json::Value>(&line.unwrap()) else { continue };
        let word = rec["word"].as_str().unwrap_or_default().to_lowercase();
        let session = rec["session"].as_str().unwrap_or_default().to_string();
        let prev = prev_word.take().filter(|(s, _)| *s == session).map(|(_, w)| w);
        prev_word = Some((session, word.clone()));
        if word.chars().count() < 2 || !word.chars().all(|c| c.is_ascii_lowercase()) {
            continue;
        }
        let cw = rec["canvas_width"].as_f64().unwrap_or(1.0) as f32;
        let ch = rec["canvas_height"].as_f64().unwrap_or(1.0) as f32;
        let kw = cw * 0.1; // ten keys per row
        let keys = KeyMap::new(layout["keys"].as_array().unwrap().iter().map(|k| {
            let c = k["letter"].as_str().unwrap().chars().next().unwrap();
            (c, k["cx"].as_f64().unwrap() as f32 * cw / kw, k["cy"].as_f64().unwrap() as f32 * ch / kw)
        }));
        let raw: Vec<(f32, f32, f32)> = rec["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["x"].as_f64().unwrap() as f32 * cw / kw,
                    p["y"].as_f64().unwrap() as f32 * ch / kw,
                    p["t"].as_f64().unwrap_or(0.0) as f32,
                )
            })
            .collect();
        let t0 = raw.first().map(|p| p.2).unwrap_or(0.0);
        let raw: Vec<(f32, f32, f32)> = raw.into_iter().map(|(x, y, t)| (x, y, t - t0)).collect();
        let (pts, times) = clean_trace_timed(&raw, 0.05);
        if langs[0].dict.as_ref().unwrap().lookup_any_case(&word).is_none() {
            oov += 1;
            continue;
        }
        total += 1;
        let t = Instant::now();
        let ctx = if use_ctx { prev.as_deref() } else { None };
        let r = dec.decode_timed(&pts, Some(&times), &keys, &langs, &[(0, 0.0)], ctx);
        lat.push(t.elapsed().as_secs_f64() * 1000.0);
        let pos = r.iter().position(|c| c.word.to_lowercase() == word);
        if pos == Some(0) {
            top1 += 1;
        }
        if pos.map(|p| p < 3).unwrap_or(false) {
            top3 += 1;
        } else if misses.len() < 15 {
            misses.push(format!("{word} -> {:?}", r.iter().take(3).map(|c| c.word.as_str()).collect::<Vec<_>>()));
        }
    }
    lat.sort_by(|a, b| a.total_cmp(b));
    println!(
        "[{spec}] FUTO real gestures: {total} in-vocabulary words ({oov} OOV skipped): top1 {:.1}%  top3 {:.1}%  p50 {:.1} ms  p95 {:.1} ms",
        100.0 * top1 as f64 / total as f64,
        100.0 * top3 as f64 / total as f64,
        lat[lat.len() / 2],
        lat[lat.len() * 95 / 100]
    );
    if quiet {
        return;
    }
    for m in misses {
        println!("  miss: {m}");
    }
}
