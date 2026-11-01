//! Glide decoder benchmark on synthetic gestures over real dictionaries.
//! `cargo run --release -p glissando-swipe --example bench -- [lang] [words] [noise]`

use glissando_core::geometry::{GeometryParams, LayerGeometry, Rect};
use glissando_core::layout::{Layout, SymbolSet};
use glissando_lang::dict::Dict;
use glissando_lang::model::Language;
use glissando_lang::user::UserModel;
use glissando_swipe::{KeyMap, ShapeDecoder, synth_gesture};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = args.get(1).map(|s| s.as_str()).unwrap_or("en");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let noise: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.25);
    let root = std::env::var("GLISSANDO_DATA_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../data").to_string());
    let sym = SymbolSet::parse(&std::fs::read_to_string(format!("{root}/layouts/symbols.toml")).unwrap()).unwrap();
    let layout = Layout::parse(&std::fs::read_to_string(format!("{root}/layouts/{lang}.toml")).unwrap(), &sym).unwrap();
    let layer = layout.layer("letters").unwrap();
    let mut p = GeometryParams::phone(360.0, 52.0);
    p.area = Rect::new(0.0, 0.0, 360.0, 0.0);
    let g = LayerGeometry::compute(layer, &p);
    let defs: Vec<_> = layer.keys().filter(|k| k.style != glissando_core::layout::KeyStyle::Invisible).collect();
    let keys = KeyMap::new(defs.iter().zip(&g.keys).filter_map(|(d, kg)| {
        let c = d.action.letter()?;
        let (x, y) = kg.visual.center();
        Some((c, x / g.unit, y / g.unit))
    }));
    let dict = Dict::open(std::path::Path::new(&format!("{root}/dict/{lang}.gld"))).unwrap();
    // Sample words: frequent words of length >= 2 typeable on the layout.
    let mut words: Vec<(String, u8)> = Vec::new();
    dict.for_each_word(|_, w, f| {
        if w.chars().count() >= 2 && w.chars().all(|c| c.is_alphabetic()) {
            words.push((w.to_string(), f));
        }
    });
    words.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    words.truncate(6000);
    let step = (words.len() / n).max(1);
    let sample: Vec<&(String, u8)> = words.iter().step_by(step).take(n).collect();
    let langs = vec![Language::new(lang, Some(dict), UserModel::in_memory())];
    let dec = ShapeDecoder::default();
    let (mut top1, mut top3, mut times) = (0, 0, Vec::new());
    let mut misses = Vec::new();
    for (i, (w, _)) in sample.iter().enumerate() {
        let gest = synth_gesture(w, &keys, noise, i as u64 + 1);
        let t = Instant::now();
        let r = dec.decode(&gest, &keys, &langs, &[(0, 0.0)], None);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        let lw = w.to_lowercase();
        let pos = r.iter().position(|c| c.word.to_lowercase() == lw);
        if pos == Some(0) {
            top1 += 1;
        }
        if pos.map(|p| p < 3).unwrap_or(false) {
            top3 += 1;
        } else if misses.len() < 12 {
            misses.push(format!("{w} -> {:?}", r.iter().take(3).map(|c| c.word.as_str()).collect::<Vec<_>>()));
        }
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let m = sample.len() as f64;
    println!(
        "{lang}: {} words, noise {noise}: top1 {:.1}%  top3 {:.1}%  latency p50 {:.1} ms  p95 {:.1} ms  max {:.1} ms",
        sample.len(),
        100.0 * top1 as f64 / m,
        100.0 * top3 as f64 / m,
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times.last().unwrap()
    );
    for x in misses {
        println!("  miss: {x}");
    }
}
