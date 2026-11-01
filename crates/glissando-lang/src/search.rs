//! Error tolerant dictionary search.
//!
//! The typed word is a sequence of [`InChar`]s, each carrying the costs of
//! the keys the finger could have meant (from the spatial model). The search
//! walks the trie with a bounded weighted edit distance (substitution,
//! omission, insertion, transposition) and also yields completions of
//! promising prefixes. Costs are in nats: lower is better.

use crate::dict::Dict;
use glissando_core::text::fold_char;
use std::collections::{BinaryHeap, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub struct InChar {
    /// The character the key produced (lowercase).
    pub typed: char,
    /// Plausible intended characters (lowercase) with their spatial cost.
    pub alts: Vec<(char, f32)>,
}

impl InChar {
    pub fn exact(c: char) -> InChar {
        let l = c.to_lowercase().next().unwrap_or(c);
        InChar { typed: l, alts: vec![(l, 0.0)] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Costs {
    pub substitute: f32,
    pub omit: f32,
    pub insert: f32,
    pub transpose: f32,
    /// Typing a letter without its diacritic (u → ü).
    pub diacritic: f32,
    /// Cost of completing a prefix, plus per extra character.
    pub complete: f32,
    pub complete_per_char: f32,
    /// Omitting or repeating a doubled letter ("helo", "helllo").
    pub double: f32,
}

impl Default for Costs {
    fn default() -> Self {
        Costs {
            substitute: 4.5,
            omit: 3.6,
            insert: 3.6,
            transpose: 2.6,
            diacritic: 0.5,
            complete: 0.6,
            complete_per_char: 0.12,
            double: 1.3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub node: u32,
    pub cost: f32,
    pub freq: u8,
    /// True when the word is longer than the input (prefix completion).
    pub completion: bool,
}

pub struct SearchOpts {
    pub costs: Costs,
    pub max_cost: f32,
    pub completions: bool,
    /// Prefixes cheaper than this are completed.
    pub completion_max_cost: f32,
    pub completions_per_prefix: usize,
    pub max_results: usize,
}

impl SearchOpts {
    /// Budget scales with word length: short words allow fewer edits.
    pub fn for_len(n: usize) -> SearchOpts {
        let costs = Costs::default();
        let max_cost = match n {
            0..=1 => 1.2,
            2 => 2.9,
            3..=4 => costs.substitute + 0.6,
            _ => costs.substitute + costs.omit + 0.8,
        };
        SearchOpts {
            costs,
            max_cost,
            completions: true,
            completion_max_cost: if n <= 2 { 1.6 } else { 3.0 },
            completions_per_prefix: 4,
            max_results: 40,
        }
    }
}

/// Cost of matching trie character `c` against input position `ic`.
fn match_cost(ic: &InChar, c: char, costs: &Costs) -> Option<f32> {
    let lower = c.to_lowercase().next().unwrap_or(c);
    for &(a, cost) in &ic.alts {
        if a == lower {
            return Some(cost);
        }
    }
    let folded = fold_char(lower);
    if folded != lower {
        for &(a, cost) in &ic.alts {
            if a == folded {
                let d = if lower == 'ё' { 0.05 } else { costs.diacritic };
                return Some(cost + d);
            }
        }
    }
    None
}

pub fn search(dict: &Dict, input: &[InChar], opts: &SearchOpts) -> Vec<Match> {
    let n = input.len();
    let c = &opts.costs;
    let mut best: HashMap<u32, Match> = HashMap::new();
    let mut seen: HashMap<(u32, u16), f32> = HashMap::new();
    let mut prefixes: Vec<(u32, f32)> = Vec::new();
    // (node, input index, cost)
    let mut stack: Vec<(u32, usize, f32)> = vec![(0, 0, 0.0)];
    while let Some((id, i, cost)) = stack.pop() {
        if cost > opts.max_cost {
            continue;
        }
        let key = (id, i as u16);
        if let Some(&prev) = seen.get(&key) {
            if prev <= cost {
                continue;
            }
        }
        seen.insert(key, cost);
        let node = dict.node(id);
        if i == n {
            if node.freq > 0 {
                let m = Match { node: id, cost, freq: node.freq, completion: false };
                match best.get(&id) {
                    Some(b) if b.cost <= cost => {}
                    _ => {
                        best.insert(id, m);
                    }
                }
            }
            if opts.completions && cost <= opts.completion_max_cost && id != 0 {
                prefixes.push((id, cost));
            }
        }
        // Typed an extra character: skip input[i].
        if i < n {
            let repeated = i > 0 && input[i].typed == input[i - 1].typed;
            stack.push((id, i + 1, cost + if repeated { c.double } else { c.insert }));
        }
        let node_lower = fold_char(node.ch);
        for (cid, child) in dict.children(&node) {
            if child.max_freq == 0 {
                continue;
            }
            if i < n {
                if let Some(mc) = match_cost(&input[i], child.ch, c) {
                    stack.push((cid, i + 1, cost + mc));
                } else {
                    stack.push((cid, i + 1, cost + c.substitute));
                }
                // Transposition: child matches input[i+1] and grandchild matches input[i].
                if i + 1 < n {
                    if let Some(m1) = match_cost(&input[i + 1], child.ch, c) {
                        for (gid, g) in dict.children(&child) {
                            if let Some(m0) = match_cost(&input[i], g.ch, c) {
                                stack.push((gid, i + 2, cost + c.transpose + m0 + m1));
                            }
                        }
                    }
                }
            }
            // Omitted a character of the word (but not past the end: that is a completion).
            if i < n {
                let doubled = id != 0 && fold_char(child.ch) == node_lower;
                stack.push((cid, i, cost + if doubled { c.double } else { c.omit }));
            }
        }
    }
    // Completions: best-first by subtree max frequency.
    if opts.completions {
        prefixes.sort_by(|a, b| a.1.total_cmp(&b.1));
        prefixes.truncate(24);
        for (pid, pcost) in prefixes {
            for (id, extra) in top_completions(dict, pid, opts.completions_per_prefix) {
                if extra == 0 {
                    continue;
                }
                let node = dict.node(id);
                let cost = pcost + c.complete + c.complete_per_char * extra as f32;
                match best.get(&id) {
                    Some(b) if b.cost <= cost => {}
                    _ => {
                        best.insert(id, Match { node: id, cost, freq: node.freq, completion: true });
                    }
                }
            }
        }
    }
    let mut v: Vec<Match> = best.into_values().collect();
    v.sort_by(|a, b| a.cost.total_cmp(&b.cost).then(b.freq.cmp(&a.freq)));
    v.truncate(opts.max_results.max(1) * 4);
    v
}

/// The `k` most frequent words in the subtree of `root`, with their depth
/// below the root.
pub fn top_completions(dict: &Dict, root: u32, k: usize) -> Vec<(u32, usize)> {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Item(u8, bool, std::cmp::Reverse<u32>, usize);
    let mut heap = BinaryHeap::new();
    let r = dict.node(root);
    heap.push(Item(r.max_freq, false, std::cmp::Reverse(root), 0));
    let mut out = Vec::new();
    let mut pops = 0;
    while let Some(Item(f, terminal, std::cmp::Reverse(id), depth)) = heap.pop() {
        pops += 1;
        if pops > 4000 {
            break;
        }
        if terminal {
            out.push((id, depth));
            if out.len() == k {
                break;
            }
            continue;
        }
        let n = dict.node(id);
        if n.freq > 0 {
            heap.push(Item(n.freq, true, std::cmp::Reverse(id), depth));
        }
        for (cid, c) in dict.children(&n) {
            if c.max_freq > 0 {
                heap.push(Item(c.max_freq, false, std::cmp::Reverse(cid), depth + 1));
            }
        }
        let _ = f;
    }
    out
}

/// Spatial model: converts a tap into candidate characters with costs.
#[derive(Debug, Clone, Default)]
pub struct SpatialModel {
    /// Lowercase character and key centre in key-width units.
    keys: Vec<(char, f32, f32)>,
    sigma: f32,
}

impl SpatialModel {
    pub fn new(keys: Vec<(char, f32, f32)>) -> SpatialModel {
        let keys = keys.into_iter().map(|(c, x, y)| (c.to_lowercase().next().unwrap_or(c), x, y)).collect();
        SpatialModel { keys, sigma: 0.55 }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn center(&self, c: char) -> Option<(f32, f32)> {
        let l = c.to_lowercase().next().unwrap_or(c);
        self.keys.iter().find(|k| k.0 == l).map(|k| (k.1, k.2))
    }

    /// Candidates for a tap that produced `typed` at `touch` (key units).
    pub fn input_char(&self, typed: char, touch: Option<(f32, f32)>) -> InChar {
        let typed_l = typed.to_lowercase().next().unwrap_or(typed);
        let Some(origin) = touch.or_else(|| self.center(typed_l)) else {
            return InChar::exact(typed_l);
        };
        let two_s2 = 2.0 * self.sigma * self.sigma;
        let mut alts: Vec<(char, f32)> = self
            .keys
            .iter()
            .filter_map(|&(c, x, y)| {
                let d2 = (x - origin.0).powi(2) + (y - origin.1).powi(2);
                (d2 < 2.6).then_some((c, d2 / two_s2))
            })
            .collect();
        if touch.is_none() {
            // Without a touch point only direct neighbours are plausible.
            for a in alts.iter_mut() {
                if a.0 != typed_l {
                    a.1 = a.1.max(1.8);
                }
            }
        }
        let typed_cost = alts.iter().find(|a| a.0 == typed_l).map(|a| a.1);
        match typed_cost {
            Some(tc) => {
                for a in alts.iter_mut() {
                    a.1 = (a.1 - tc).max(0.0);
                    if a.0 != typed_l {
                        // Users hit the key they see most of the time.
                        a.1 += 0.7;
                    }
                }
            }
            None => alts.push((typed_l, 0.0)),
        }
        alts.sort_by(|a, b| a.1.total_cmp(&b.1));
        InChar { typed: typed_l, alts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::test_dict;

    fn qwerty() -> SpatialModel {
        let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
        let offs = [0.0, 0.5, 1.5];
        let mut keys = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                keys.push((c, offs[r] + i as f32 + 0.5, r as f32 * 1.3 + 0.65));
            }
        }
        SpatialModel::new(keys)
    }

    fn words(d: &crate::dict::Dict, m: &[Match]) -> Vec<String> {
        m.iter().map(|m| d.word(m.node)).collect()
    }

    fn typed(sm: &SpatialModel, s: &str) -> Vec<InChar> {
        s.chars().map(|c| sm.input_char(c, None)).collect()
    }

    #[test]
    fn finds_neighbour_typos_and_transpositions() {
        let d = test_dict(
            &[("hello", 200), ("help", 190), ("the", 230), ("there", 200), ("jello", 90), ("über", 150)],
            &[],
        );
        let sm = qwerty();
        // 'g' is next to 'h'.
        let m = search(&d, &typed(&sm, "gello"), &SearchOpts::for_len(5));
        assert_eq!(words(&d, &m)[0], "hello");
        let m = search(&d, &typed(&sm, "teh"), &SearchOpts::for_len(3));
        assert!(words(&d, &m).contains(&"the".to_string()));
        let m = search(&d, &typed(&sm, "uber"), &SearchOpts::for_len(4));
        assert_eq!(words(&d, &m)[0], "über");
        // Completion of a prefix.
        let m = search(&d, &typed(&sm, "hel"), &SearchOpts::for_len(3));
        let w = words(&d, &m);
        assert!(w.contains(&"hello".to_string()) && w.contains(&"help".to_string()));
        // Omitted letter.
        let m = search(&d, &typed(&sm, "helo"), &SearchOpts::for_len(4));
        assert_eq!(words(&d, &m)[0], "hello");
    }

    #[test]
    fn spatial_prefers_touch_point() {
        let sm = qwerty();
        let (hx, hy) = sm.center('h').unwrap();
        let (gx, _) = sm.center('g').unwrap();
        // A tap on the h/g boundary that produced 'h' makes 'g' nearly as likely.
        let ic = sm.input_char('h', Some(((hx + gx) / 2.0 + 0.05, hy)));
        let g = ic.alts.iter().find(|a| a.0 == 'g').unwrap().1;
        assert!(g < 1.0, "{g}");
        let ic = sm.input_char('h', Some((hx, hy)));
        let g = ic.alts.iter().find(|a| a.0 == 'g').unwrap().1;
        assert!(g > 2.0, "{g}");
    }

    #[test]
    fn top_completions_by_frequency() {
        let d = test_dict(&[("car", 100), ("cart", 50), ("care", 180), ("careful", 120)], &[]);
        let (car, _) = d.lookup("car").unwrap();
        let c = top_completions(&d, car, 3);
        let w: Vec<String> = c.iter().map(|(id, _)| d.word(*id)).collect();
        assert_eq!(w, vec!["care", "careful", "car"]);
    }
}
