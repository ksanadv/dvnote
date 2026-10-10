// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::text::{Mapped, Text};
use ropey::{Rope, RopeBuilder};
use std::io::{self, Write};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Crlf,
    Lf,
}

impl LineEnding {
    pub const NATIVE: LineEnding = if cfg!(windows) { LineEnding::Crlf } else { LineEnding::Lf };

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Crlf => "Windows (CRLF)",
            LineEnding::Lf => "Unix (LF)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

impl Encoding {
    pub const ALL: [Encoding; 5] =
        [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Ansi];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Ansi => "ANSI",
        }
    }
}

const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0,
    0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

fn decode_ansi(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => {
                char::from_u32(CP1252_HIGH[(b - 0x80) as usize] as u32).filter(|&c| c != '\0').unwrap_or('\u{FFFD}')
            }
            _ => b as char,
        })
        .collect()
}

fn ansi_byte(c: char) -> u8 {
    match c as u32 {
        0..=0x7F | 0xA0..=0xFF => c as u32 as u8,
        u => CP1252_HIGH.iter().position(|&h| h as u32 == u && h != 0).map_or(b'?', |i| 0x80 + i as u8),
    }
}

fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let units = bytes.chunks(2).map(|p| match (p, little) {
        ([a, b], true) => u16::from_le_bytes([*a, *b]),
        ([a, b], false) => u16::from_be_bytes([*a, *b]),
        ([a], _) => *a as u16, // dangling odd byte
        _ => 0,
    });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

pub struct Loaded {
    pub text: Text,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

const PIECE: usize = 1 << 20;
const PARALLEL_MIN: usize = 2 << 20;
const MAX_THREADS: usize = 8;

// \r\n and lone \r become \n
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('\r') {
        out.push_str(&rest[..p]);
        if !rest[p + 1..].starts_with('\n') {
            out.push('\n');
        }
        rest = &rest[p + 1..];
    }
    out.push_str(rest);
    out
}

fn build_rope(text: &str) -> Rope {
    if !text.contains('\r') {
        return Rope::from_str(text);
    }
    let mut b = RopeBuilder::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + PIECE).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        // keep CRLF pairs in one piece
        if end < text.len() && bytes[end - 1] == b'\r' && bytes[end] == b'\n' {
            end += 1;
        }
        b.append(&normalize(&text[start..end]));
        start = end;
    }
    b.finish()
}

// One chunk per thread, each ending right after a '\n': a boundary is always a char boundary and never splits a \r\n.
pub(crate) fn chunk_cuts(bytes: &[u8]) -> Vec<usize> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(MAX_THREADS);
    let mut cuts = vec![0];
    for i in 1..threads {
        let from = (bytes.len() / threads * i).max(*cuts.last().unwrap());
        if let Some(p) = bytes[from..].iter().position(|&b| b == b'\n') {
            cuts.push(from + p + 1);
        }
    }
    cuts.push(bytes.len());
    cuts.dedup();
    cuts
}

// Validate and build the rope on several threads. Chunks end right after a '\n', so a chunk boundary is always a
// character boundary and never splits a \r\n pair. None means the bytes are not valid UTF-8.
pub(crate) fn rope_from_utf8(bytes: &[u8]) -> Option<Rope> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(MAX_THREADS);
    if bytes.len() < PARALLEL_MIN || threads < 2 {
        return Some(build_rope(std::str::from_utf8(bytes).ok()?));
    }
    let cuts = chunk_cuts(bytes);
    let parts: Vec<Option<Rope>> = std::thread::scope(|scope| {
        let handles: Vec<_> = cuts
            .windows(2)
            .map(|w| {
                let chunk = &bytes[w[0]..w[1]];
                scope.spawn(move || std::str::from_utf8(chunk).ok().map(build_rope))
            })
            .collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut rope = Rope::new();
    for part in parts {
        rope.append(part?);
    }
    Some(rope)
}

fn detect_line_ending(utf8: &[u8]) -> LineEnding {
    match utf8.iter().position(|&b| b == b'\n') {
        Some(i) if i > 0 && utf8[i - 1] == b'\r' => LineEnding::Crlf,
        Some(_) => LineEnding::Lf,
        None if utf8.contains(&b'\r') => LineEnding::Lf,
        None => LineEnding::NATIVE,
    }
}

pub fn decode(bytes: &[u8]) -> Loaded {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let rope = rope_from_utf8(rest).unwrap_or_else(|| build_rope(&String::from_utf8_lossy(rest)));
        return Loaded { text: Text::Rope(rope), encoding: Encoding::Utf8Bom, line_ending: detect_line_ending(rest) };
    }
    let from_text = |text: String, encoding| Loaded {
        line_ending: detect_line_ending(text.as_bytes()),
        text: Text::Rope(build_rope(&text)),
        encoding,
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return from_text(decode_utf16(rest, true), Encoding::Utf16Le);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return from_text(decode_utf16(rest, false), Encoding::Utf16Be);
    }
    match rope_from_utf8(bytes) {
        Some(rope) => {
            Loaded { text: Text::Rope(rope), encoding: Encoding::Utf8, line_ending: detect_line_ending(bytes) }
        }
        None => from_text(decode_ansi(bytes), Encoding::Ansi),
    }
}

pub fn load(path: &Path) -> io::Result<Loaded> {
    // a large local UTF-8 file is mapped instead of read: no copy, and the memory is only the pages that are looked at
    if let Some(m) = Mapped::open(path) {
        let encoding = if m.has_bom() { Encoding::Utf8Bom } else { Encoding::Utf8 };
        let line_ending = if m.is_crlf() {
            LineEnding::Crlf
        } else if m.lines() > 1 {
            LineEnding::Lf
        } else {
            LineEnding::NATIVE
        };
        return Ok(Loaded { text: Text::Mapped(m), encoding, line_ending });
    }
    Ok(decode(&std::fs::read(path)?))
}

pub fn write_encoded<W: Write>(w: &mut W, rope: &Rope, encoding: Encoding, line_ending: LineEnding) -> io::Result<()> {
    match encoding {
        Encoding::Utf8Bom => w.write_all(&[0xEF, 0xBB, 0xBF])?,
        Encoding::Utf16Le => w.write_all(&[0xFF, 0xFE])?,
        Encoding::Utf16Be => w.write_all(&[0xFE, 0xFF])?,
        _ => {}
    }
    let crlf = line_ending == LineEnding::Crlf;
    let mut scratch = String::new();
    let mut out: Vec<u8> = Vec::new();
    for chunk in rope.chunks() {
        let s: &str = if crlf && chunk.contains('\n') {
            scratch.clear();
            for (i, part) in chunk.split('\n').enumerate() {
                if i > 0 {
                    scratch.push_str("\r\n");
                }
                scratch.push_str(part);
            }
            &scratch
        } else {
            chunk
        };
        match encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => w.write_all(s.as_bytes())?,
            Encoding::Utf16Le | Encoding::Utf16Be => {
                out.clear();
                for u in s.encode_utf16() {
                    out.extend_from_slice(&if encoding == Encoding::Utf16Le {
                        u.to_le_bytes()
                    } else {
                        u.to_be_bytes()
                    });
                }
                w.write_all(&out)?;
            }
            Encoding::Ansi => {
                out.clear();
                out.extend(s.chars().map(ansi_byte));
                w.write_all(&out)?;
            }
        }
    }
    Ok(())
}

pub fn save(path: &Path, rope: &Rope, encoding: Encoding, line_ending: LineEnding) -> io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.dvnote-tmp"));
    let result = (|| {
        let mut w = io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        write_encoded(&mut w, rope, encoding, line_ending)?;
        w.flush()?;
        drop(w);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(text: &str, enc: Encoding, le: LineEnding) -> Vec<u8> {
        let mut v = Vec::new();
        write_encoded(&mut v, &Rope::from_str(text), enc, le).unwrap();
        v
    }

    #[test]
    fn crlf_round_trip() {
        let bytes = b"one\r\ntwo\r\n";
        let l = decode(bytes);
        assert_eq!(l.text.to_string(), "one\ntwo\n");
        assert_eq!(l.line_ending, LineEnding::Crlf);
        assert_eq!(encode(&l.text.to_string(), l.encoding, l.line_ending), bytes);
    }

    #[test]
    fn lf_round_trip() {
        let bytes = b"# Title\n\nbody\n";
        let l = decode(bytes);
        assert_eq!((l.encoding, l.line_ending), (Encoding::Utf8, LineEnding::Lf));
        assert_eq!(encode(&l.text.to_string(), l.encoding, l.line_ending), bytes);
    }

    #[test]
    fn bom_and_utf16_round_trip() {
        for enc in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = encode("héllo\nwörld ✓", enc, LineEnding::Crlf);
            let l = decode(&bytes);
            assert_eq!(l.encoding, enc);
            assert_eq!(l.text.to_string(), "héllo\nwörld ✓");
            assert_eq!(encode(&l.text.to_string(), l.encoding, l.line_ending), bytes);
        }
    }

    #[test]
    fn ansi_fallback() {
        let bytes = b"caf\xE9 \x93quoted\x94 \x80";
        let l = decode(bytes);
        assert_eq!(l.encoding, Encoding::Ansi);
        assert_eq!(l.text.to_string(), "café \u{201C}quoted\u{201D} €");
        assert_eq!(encode(&l.text.to_string(), l.encoding, LineEnding::Lf), bytes);
    }

    #[test]
    fn empty_file_defaults_to_crlf_on_windows() {
        let l = decode(b"");
        assert_eq!(l.line_ending, if cfg!(windows) { LineEnding::Crlf } else { LineEnding::Lf });
    }

    #[test]
    fn lone_cr_becomes_lf() {
        let l = decode(b"a\rb\rc");
        assert_eq!(l.text.to_string(), "a\nb\nc");
        assert_eq!(l.line_ending, LineEnding::Lf);
    }

    #[test]
    fn crlf_normalization_across_piece_boundaries() {
        let mut s = "x".repeat(PIECE - 1);
        s.push_str("\r\n");
        s.push_str("tail\r\n");
        let l = decode(s.as_bytes());
        let t = l.text.to_string();
        assert_eq!(t.len(), PIECE - 1 + 1 + 4 + 1);
        assert!(!t.contains('\r'));
        assert_eq!(l.text.len_lines(), 3);
    }

    #[test]
    fn save_replaces_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("dvnote-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.md");
        std::fs::write(&p, "old").unwrap();
        save(&p, &Rope::from_str("new\ntext"), Encoding::Utf8, LineEnding::Crlf).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new\r\ntext");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdown" | "mkd"))
}

#[cfg(test)]
mod markdown_tests {
    use super::*;

    #[test]
    fn markdown_extensions() {
        assert!(is_markdown(Path::new("notes/README.MD")));
        assert!(is_markdown(Path::new("a.markdown")));
        assert!(!is_markdown(Path::new("a.txt")));
        assert!(!is_markdown(Path::new("md")));
    }
}

#[cfg(test)]
mod parallel_tests {
    use super::*;

    fn sample(lines: usize, crlf: bool) -> Vec<u8> {
        let nl = if crlf { "\r\n" } else { "\n" };
        (0..lines)
            .map(|i| format!("line {i}: héllo wörld 日本語 {}{nl}", "x".repeat(i % 37)))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn parallel_matches_sequential_for_lf_and_crlf() {
        for crlf in [false, true] {
            let bytes = sample(120_000, crlf);
            assert!(bytes.len() > PARALLEL_MIN);
            let fast = decode(&bytes);
            let expected = String::from_utf8(bytes.clone()).unwrap().replace("\r\n", "\n");
            assert_eq!(fast.text.to_string(), expected);
            assert_eq!(fast.encoding, Encoding::Utf8);
            assert_eq!(fast.line_ending, if crlf { LineEnding::Crlf } else { LineEnding::Lf });
            assert_eq!(fast.text.len_lines(), expected.matches('\n').count() + 1);
        }
    }

    #[test]
    fn lone_cr_and_mixed_endings_survive_chunking() {
        let mut bytes = sample(100_000, true);
        bytes.extend_from_slice(b"a\rb\r\nc\nd\r");
        let fast = decode(&bytes);
        let s = fast.text.to_string();
        assert!(!s.contains('\r'));
        assert!(s.ends_with("a\nb\nc\nd\n"));
    }

    #[test]
    fn invalid_utf8_in_a_late_chunk_falls_back_to_ansi() {
        let mut bytes = sample(100_000, false);
        let at = bytes.len() - 10;
        bytes[at] = 0xE9;
        let loaded = decode(&bytes);
        assert_eq!(loaded.encoding, Encoding::Ansi);
        assert_eq!(loaded.text.len_lines(), 100_001);
    }

    #[test]
    fn one_giant_line_has_no_split_point() {
        let bytes = "ab ".repeat(2_000_000).into_bytes();
        let loaded = decode(&bytes);
        assert_eq!(loaded.text.len_chars(), 6_000_000);
        assert_eq!(loaded.text.len_lines(), 1);
    }

    #[test]
    fn normalize_handles_every_cr_shape() {
        assert_eq!(normalize("a\r\nb\rc\n\r\rd\r"), "a\nb\nc\n\n\nd\n");
        assert_eq!(normalize("plain"), "plain");
        assert_eq!(normalize("\r\n\r\n"), "\n\n");
    }
}
