// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> TempFile {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "dvnote-text-{}-{}.txt",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::File::create(&path).unwrap().write_all(bytes).unwrap();
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn mapped(bytes: &[u8]) -> (TempFile, Option<Mapped>) {
    let f = TempFile::new(bytes);
    let m = Mapped::open_min(&f.0, 1);
    (f, m)
}

// Every observable of a mapped text must equal the rope built from the same text with "\r\n" read as "\n".
fn assert_same(src: &str) {
    let (_f, m) = mapped(src.as_bytes());
    let text = Text::Mapped(m.expect("should map"));
    let expect = Text::from_str(&src.replace("\r\n", "\n"));
    assert_eq!(text.len_chars(), expect.len_chars(), "len_chars of {src:?}");
    assert_eq!(text.len_lines(), expect.len_lines(), "len_lines of {src:?}");
    for line in 0..expect.len_lines() {
        assert_eq!(text.line_to_char(line), expect.line_to_char(line), "line_to_char {line} of {src:?}");
        assert_eq!(text.line_len(line), expect.line_len(line), "line_len {line} of {src:?}");
        assert_eq!(text.line_text(line), expect.line_text(line), "line_text {line} of {src:?}");
    }
    let n = expect.len_chars();
    for i in 0..=n {
        assert_eq!(text.char_to_line(i), expect.char_to_line(i), "char_to_line {i} of {src:?}");
        assert_eq!(
            text.chars_from(i).collect::<String>(),
            expect.chars_from(i).collect::<String>(),
            "chars_from {i} of {src:?}"
        );
        assert_eq!(
            text.chars_before(i).collect::<String>(),
            expect.chars_before(i).collect::<String>(),
            "chars_before {i} of {src:?}"
        );
        for j in i..=n {
            assert_eq!(text.slice(i, j), expect.slice(i, j), "slice {i}..{j} of {src:?}");
        }
    }
    for i in 0..n {
        assert_eq!(text.char_at(i), expect.char_at(i), "char_at {i} of {src:?}");
    }
    assert_eq!(text.to_string(), expect.to_string());
}

#[test]
fn matches_a_rope_for_lf_text() {
    for s in ["a", "a\n", "\n", "\n\n", "one\ntwo\nthree", "one\ntwo\nthree\n", "\n\nx\n\n"] {
        assert_same(s);
    }
}

#[test]
fn matches_a_rope_for_crlf_text() {
    for s in ["a\r\nb", "a\r\nb\r\n", "\r\n", "\r\n\r\n", "x\r\n\r\ny"] {
        assert_same(s);
    }
}

#[test]
fn matches_a_rope_for_mixed_line_endings() {
    assert_same("a\r\nb\nc\r\nd");
    assert_same("a\nb\r\nc\n");
}

#[test]
fn matches_a_rope_for_multibyte_text() {
    assert_same("héllo\nwörld\r\nשלום עולם\n😀 emoji 😀\r\n日本語\n");
    assert_same("😀😀😀\n😀\r\n");
}

#[test]
fn matches_a_rope_across_many_chunks() {
    // enough lines that every thread gets its own chunk
    let mut s = String::new();
    for i in 0..3000 {
        s.push_str(&format!("line {i} é😀{}", if i % 3 == 0 { "\r\n" } else { "\n" }));
    }
    let (_f, m) = mapped(s.as_bytes());
    let text = Text::Mapped(m.unwrap());
    let expect = Text::from_str(&s.replace("\r\n", "\n"));
    assert_eq!(text.len_chars(), expect.len_chars());
    assert_eq!(text.len_lines(), expect.len_lines());
    for line in 0..expect.len_lines() {
        assert_eq!(text.line_to_char(line), expect.line_to_char(line));
        assert_eq!(text.line_text(line), expect.line_text(line));
    }
    assert_eq!(text.to_string(), expect.to_string());
}

// ASCII files skip the char table when every break is the same kind; a non-ASCII chunk or mixed breaks bring it back,
// with the ASCII chunks' entries derived from their line starts
#[test]
fn ascii_and_mixed_chunks_match_a_rope() {
    let build = |f: &dyn Fn(usize) -> String| (0..3000).map(f).collect::<String>();
    let cases = [
        ("ascii lf", build(&|i| format!("line {i} plain\n"))),
        ("ascii crlf", build(&|i| format!("line {i} plain\r\n"))),
        ("ascii mixed", build(&|i| format!("line {i}{}", if i % 7 == 0 { "\r\n" } else { "\n" }))),
        ("one non-ascii line late", build(&|i| format!("line {i}{}\r\n", if i == 2900 { " é" } else { "" }))),
        ("non-ascii early, ascii after", build(&|i| format!("{}{i}\n", if i < 10 { "😀" } else { "x" }))),
    ];
    for (name, s) in cases {
        let (_f, m) = mapped(s.as_bytes());
        let m = m.expect(name);
        let formula = matches!(m.chars, CharIndex::Ascii { .. });
        assert_eq!(formula, name.starts_with("ascii ") && name != "ascii mixed", "{name}");
        let text = Text::Mapped(m);
        let expect = Text::from_str(&s.replace("\r\n", "\n"));
        assert_eq!(text.len_chars(), expect.len_chars(), "{name}");
        assert_eq!(text.len_lines(), expect.len_lines(), "{name}");
        for line in (0..expect.len_lines()).step_by(7).chain([expect.len_lines() - 1]) {
            assert_eq!(text.line_to_char(line), expect.line_to_char(line), "{name} line {line}");
            assert_eq!(text.line_len(line), expect.line_len(line), "{name} line {line}");
            assert_eq!(text.line_text(line), expect.line_text(line), "{name} line {line}");
        }
        for i in (0..=expect.len_chars()).step_by(97).chain([expect.len_chars()]) {
            assert_eq!(text.char_to_line(i), expect.char_to_line(i), "{name} char {i}");
            assert_eq!(text.char_at(i.min(expect.len_chars() - 1)), expect.char_at(i.min(expect.len_chars() - 1)));
        }
        assert_eq!(text.to_string(), expect.to_string(), "{name}");
    }
}

#[test]
fn long_line_without_breaks() {
    let s = "ab😀".repeat(5000);
    assert_same(&"ab😀".repeat(6));
    let (_f, m) = mapped(s.as_bytes());
    let text = Text::Mapped(m.unwrap());
    assert_eq!(text.len_lines(), 1);
    assert_eq!(text.line_len(0), 15000);
    assert_eq!(text.slice(2, 5), "😀ab");
}

#[test]
fn reports_bom_and_line_ending() {
    let (_f, m) = mapped(b"\xEF\xBB\xBFa\r\nb\r\n");
    let m = m.unwrap();
    assert!(m.has_bom() && m.is_crlf());
    assert_eq!(Text::Mapped(m).to_string(), "a\nb\n");
    let (_g, m) = mapped(b"a\nb\n");
    let m = m.unwrap();
    assert!(!m.has_bom() && !m.is_crlf());
}

#[test]
fn falls_back_when_the_file_cannot_be_read_as_is() {
    assert!(mapped(b"a\rb\n").1.is_none(), "lone CR");
    assert!(mapped(b"a\r").1.is_none(), "trailing CR");
    assert!(mapped(b"a\r\r\nb").1.is_none(), "CR before CRLF");
    assert!(mapped(b"ok\n\xFF\xFEbad\n").1.is_none(), "invalid UTF-8");
    assert!(mapped(b"\xFF\xFEa\0b\0").1.is_none(), "UTF-16 LE");
    assert!(mapped(b"\xFE\xFF\0a\0b").1.is_none(), "UTF-16 BE");
    assert!(mapped(b"").1.is_none(), "empty");
}

#[test]
fn size_gate() {
    let f = TempFile::new(b"hello\n");
    assert!(Mapped::open_min(&f.0, 6).is_some());
    assert!(Mapped::open_min(&f.0, 7).is_none());
    assert!(Mapped::open(&f.0).is_none(), "below MAP_MIN_BYTES");
}

#[test]
fn mapped_file_is_locked_against_writers_until_materialized() {
    let f = TempFile::new(b"alpha\r\nbeta\r\n");
    let mut text = Text::Mapped(Mapped::open_min(&f.0, 1).unwrap());
    assert!(std::fs::OpenOptions::new().write(true).open(&f.0).is_err(), "write-open must fail while mapped");
    assert!(std::fs::remove_file(&f.0).is_err(), "delete must fail while mapped");
    assert!(std::fs::File::open(&f.0).is_ok(), "readers are fine");
    text.rope_mut();
    assert!(!text.is_mapped());
    assert!(std::fs::OpenOptions::new().write(true).open(&f.0).is_ok(), "released after materializing");
}

#[test]
fn materializing_keeps_the_text_and_allows_editing() {
    let src = "héllo\r\nwörld\r\n😀\n";
    let (_f, m) = mapped(src.as_bytes());
    let mut text = Text::Mapped(m.unwrap());
    let before = text.to_string();
    let rope = text.rope_mut();
    assert_eq!(rope.to_string(), before);
    rope.insert(0, ">> ");
    assert_eq!(text.to_string(), format!(">> {before}"));
}

#[test]
fn buffer_edits_a_mapped_text_with_undo_and_redo() {
    use crate::buffer::Buffer;
    let (_f, m) = mapped(b"one\r\ntwo\r\nthree\r\n");
    let mut buf = Buffer::from_text(Text::Mapped(m.unwrap()));
    assert!(buf.text.is_mapped());
    assert_eq!((buf.len_lines(), buf.line_len(1)), (4, 3));
    let original = buf.text.to_string();
    buf.replace(4, 7, "2", crate::buffer::Kind::Other, (0, 0), (5, 5));
    assert!(!buf.text.is_mapped());
    assert_eq!(buf.text.to_string(), "one\n2\nthree\n");
    buf.undo();
    assert_eq!(buf.text.to_string(), original);
    buf.redo();
    assert_eq!(buf.text.to_string(), "one\n2\nthree\n");
}

// Timing probe, run by hand: DVNOTE_BIG=<file> cargo test --release probe -- --ignored --nocapture
#[test]
#[ignore]
fn probe() {
    let Some(path) = std::env::var_os("DVNOTE_BIG") else { return };
    let path = std::path::PathBuf::from(path);
    let t = std::time::Instant::now();
    let m = Mapped::open(&path);
    println!("open: {:?} mapped={}", t.elapsed(), m.is_some());
    if let Some(m) = m {
        let mut text = Text::Mapped(m);
        let t = std::time::Instant::now();
        let mid = text.len_chars() / 2;
        let (line, s) = (text.char_to_line(mid), text.slice(mid, mid + 20));
        println!("random access: {:?} line {line} {s:?}", t.elapsed());
        let t = std::time::Instant::now();
        text.rope_mut();
        println!("materialize: {:?}", t.elapsed());
    }
}

#[test]
fn saving_a_mapped_document_replaces_the_locked_file_and_keeps_crlf_and_bom() {
    use crate::buffer::{Buffer, Kind};
    use crate::doc::{self, Encoding, LineEnding};
    let (f, m) = mapped(b"\xEF\xBB\xBFone\r\ntwo\r\n");
    let mut buf = Buffer::from_text(Text::Mapped(m.unwrap()));
    buf.replace(0, 3, "ONE", Kind::Other, (0, 0), (3, 3));
    doc::save(&f.0, buf.rope_for_save(), Encoding::Utf8Bom, LineEnding::Crlf).unwrap();
    assert_eq!(std::fs::read(&f.0).unwrap(), b"\xEF\xBB\xBFONE\r\ntwo\r\n");
    // saving without any edit also has to release the mapping first
    let (g, m) = mapped(b"a\nb\n");
    let mut buf = Buffer::from_text(Text::Mapped(m.unwrap()));
    doc::save(&g.0, buf.rope_for_save(), Encoding::Utf8, LineEnding::Lf).unwrap();
    assert_eq!(std::fs::read(&g.0).unwrap(), b"a\nb\n");
}
