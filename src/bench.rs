// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::buffer::{Buffer, Kind};
use crate::draw::{DwLayout, Gfx};
use crate::editor::{Editor, Key, Mods};
use std::time::Instant;

fn gen(bytes: usize) -> String {
    let line = "The quick brown fox jumps over the lazy dog; fn main() { println!(\"hello\"); } // 0123456789\n";
    line.repeat(bytes / line.len())
}

fn frame(ed: &mut Editor, lay: &mut DwLayout, buf: &mut Buffer, wrap: bool) -> usize {
    ed.configure(lay, buf, wrap, 1000.0, 700.0);
    ed.visible(lay, buf).len()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

// CARGO_PROFILE_TEST_OPT_LEVEL=3 cargo test bench_ -- --ignored --nocapture --test-threads=1
#[test]
#[ignore]
fn bench_native_editor() {
    let gfx = Gfx::new().unwrap();
    for wrap in [true, false] {
        for mb in [1usize, 20, 50, 200] {
            let t = Instant::now();
            let loaded = crate::doc::decode(gen(mb << 20).as_bytes());
            let load = ms(t);
            let mut buf = Buffer::from_text(loaded.text);
            let mut lay = DwLayout::new(gfx.clone(), "Consolas", 15.0, 1.0).unwrap();
            let mut ed = Editor::new();
            let t = Instant::now();
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let first = ms(t);
            let t = Instant::now();
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let idle = ms(t);
            let t = Instant::now();
            ed.type_text(&mut buf, "x", Kind::Typing);
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let key_top = ms(t);
            let mid = buf.len_chars() / 2;
            ed.set_selection(mid, mid);
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let t = Instant::now();
            ed.type_text(&mut buf, "y", Kind::Typing);
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let key_mid = ms(t);
            let t = Instant::now();
            ed.key(&mut lay, &mut buf, Key::Down, Mods::default());
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let arrow = ms(t);
            let t = Instant::now();
            ed.key(&mut lay, &mut buf, Key::Enter, Mods::default());
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let enter = ms(t);
            println!(
                "wrap={wrap:<5} {mb:>3} MB  load {load:>6.0} | first frame {first:>6.1} idle {idle:>5.2} | key@top {key_top:>5.2} key@mid {key_mid:>5.2} arrow {arrow:>5.2} enter@mid {enter:>6.1} (ms)"
            );
        }
    }
    for wrap in [true, false] {
        for kb in [100usize, 1000, 10_000] {
            let text = "abcdefghij ".repeat(kb * 1024 / 11);
            let mut buf = Buffer::from_rope(ropey::Rope::from_str(&text));
            let mut lay = DwLayout::new(gfx.clone(), "Consolas", 15.0, 1.0).unwrap();
            let mut ed = Editor::new();
            let t = Instant::now();
            frame(&mut ed, &mut lay, &mut buf, wrap);
            let first = ms(t);
            let t = Instant::now();
            ed.type_text(&mut buf, "x", Kind::Typing);
            frame(&mut ed, &mut lay, &mut buf, wrap);
            println!("wrap={wrap:<5} one line of {kb:>6} KB | first frame {first:>6.1} | key {:>5.2} (ms)", ms(t));
        }
    }
}
