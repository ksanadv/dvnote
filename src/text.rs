// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use memmap2::Mmap;
use ropey::{iter::Chars, Rope};
use std::borrow::Cow;
use std::fmt;
use std::fs::File;
use std::path::Path;

pub const MAP_MIN_BYTES: u64 = 8 << 20;
const MAP_MAX_BYTES: u64 = 2 << 30;
const FILE_SHARE_READ: u32 = 1;
const DRIVE_FIXED: u32 = 3;
// reparse point, offline, recall on open, recall on data access (cloud placeholders)
const UNSAFE_TO_MAP: u32 = 0x400 | 0x1000 | 0x40000 | 0x400000;

/// The document text: a rope, or a read-only memory-mapped file that becomes a rope on the first edit.
/// Both present the same text: `\n` line breaks only, indexed in chars.
pub enum Text {
    Rope(Rope),
    Mapped(Mapped),
}

pub enum CharIter<'a> {
    Rope(Chars<'a>),
    Mapped { chars: std::str::Chars<'a>, backward: bool },
}

impl Iterator for CharIter<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            CharIter::Rope(c) => c.next(),
            CharIter::Mapped { chars, backward } => loop {
                // every '\r' in a mapped file is half of a "\r\n" pair, which reads as one '\n'
                let c = if *backward { chars.next_back() } else { chars.next() }?;
                if c != '\r' {
                    return Some(c);
                }
            },
        }
    }
}

impl Text {
    pub fn empty() -> Text {
        Text::Rope(Rope::new())
    }

    #[cfg(test)]
    pub fn from_str(s: &str) -> Text {
        Text::Rope(Rope::from_str(s))
    }

    pub fn is_mapped(&self) -> bool {
        matches!(self, Text::Mapped(_))
    }

    pub fn len_chars(&self) -> usize {
        match self {
            Text::Rope(r) => r.len_chars(),
            Text::Mapped(m) => m.total_chars(),
        }
    }

    pub fn len_lines(&self) -> usize {
        match self {
            Text::Rope(r) => r.len_lines(),
            Text::Mapped(m) => m.lines(),
        }
    }

    pub fn char_to_line(&self, idx: usize) -> usize {
        match self {
            Text::Rope(r) => r.char_to_line(idx.min(r.len_chars())),
            Text::Mapped(m) => m.char_to_line(idx),
        }
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        match self {
            Text::Rope(r) => r.line_to_char(line.min(r.len_lines() - 1)),
            Text::Mapped(m) => m.line_to_char(line),
        }
    }

    /// Chars in `line`, not counting its line break.
    pub fn line_len(&self, line: usize) -> usize {
        match self {
            Text::Rope(r) => {
                let l = r.line(line);
                let n = l.len_chars();
                if n > 0 && l.char(n - 1) == '\n' {
                    n - 1
                } else {
                    n
                }
            }
            Text::Mapped(m) => m.line_len(line),
        }
    }

    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        match self {
            Text::Rope(r) => {
                let mut s = r.line(line).to_string();
                if s.ends_with('\n') {
                    s.pop();
                }
                Cow::Owned(s)
            }
            Text::Mapped(m) => Cow::Borrowed(m.line_str(line)),
        }
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        match self {
            Text::Rope(r) => r.slice(a..b).to_string(),
            Text::Mapped(m) => m.slice(a, b),
        }
    }

    pub fn chars_from(&self, idx: usize) -> CharIter<'_> {
        match self {
            Text::Rope(r) => CharIter::Rope(r.chars_at(idx)),
            Text::Mapped(m) => CharIter::Mapped { chars: m.str_from(idx).chars(), backward: false },
        }
    }

    /// The chars before `idx`, nearest first.
    pub fn chars_before(&self, idx: usize) -> CharIter<'_> {
        match self {
            Text::Rope(r) => {
                let mut it = r.chars_at(idx);
                it.reverse();
                CharIter::Rope(it)
            }
            Text::Mapped(m) => CharIter::Mapped { chars: m.str_before(idx).chars(), backward: true },
        }
    }

    pub fn char_at(&self, idx: usize) -> char {
        self.chars_from(idx).next().unwrap_or('\0')
    }

    /// Make the text editable, releasing the mapped file. A no-op for a rope.
    pub fn rope_mut(&mut self) -> &mut Rope {
        if let Text::Mapped(m) = self {
            let rope = m.to_rope();
            *self = Text::Rope(rope);
        }
        match self {
            Text::Rope(r) => r,
            Text::Mapped(_) => unreachable!(),
        }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Text::Rope(r) => r.chunks().try_for_each(|c| f.write_str(c)),
            Text::Mapped(m) => f.write_str(&m.slice(0, m.total_chars())),
        }
    }
}

/// A read-only UTF-8 file mapped into memory with a per-line index. The file stays open with write and delete sharing
/// denied, so nothing can truncate or change it under the mapping.
pub struct Mapped {
    map: Mmap,
    _file: File,
    skip: usize,
    // byte offset of each line start, then the data length
    starts: Vec<u32>,
    chars: CharIndex,
    crlf: bool,
}

// where each line starts in chars, and the total after the last line
enum CharIndex {
    Table(Vec<u32>),
    // ASCII with one kind of line break: a line's char index is its byte offset less one per earlier "\r\n", so no
    // table is needed
    Ascii { crlf: bool },
}

fn count_chars(bytes: &[u8]) -> u32 {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count() as u32
}

struct Chunk {
    starts: Vec<u32>,
    // empty for an ASCII chunk, which does not need it unless the whole file turns out to
    cum: Vec<u32>,
    chars: u32,
    ascii: bool,
    breaks: u32,
    crlfs: u32,
}

// Lines that begin inside `chunk`, which ends right after a '\n' (or at the end of the data). None if the chunk is not
// valid UTF-8 or contains a '\r' that is not followed by '\n'.
fn index_chunk(chunk: &[u8], last: bool) -> Option<Chunk> {
    let ascii = chunk.is_ascii();
    if !ascii {
        std::str::from_utf8(chunk).ok()?;
    }
    let (mut starts, mut cum) = (vec![0u32], if ascii { Vec::new() } else { vec![0u32] });
    let (mut chars, mut pos, mut breaks, mut crlfs) = (0u32, 0usize, 0u32, 0u32);
    while let Some(off) = chunk[pos..].iter().position(|&b| b == b'\n') {
        let end = pos + off;
        let seg = &chunk[pos..=end];
        let crlf = end > pos && chunk[end - 1] == b'\r';
        if seg.iter().filter(|&&b| b == b'\r').count() != crlf as usize {
            return None;
        }
        chars += if ascii { seg.len() as u32 } else { count_chars(seg) } - crlf as u32;
        (breaks, crlfs) = (breaks + 1, crlfs + crlf as u32);
        pos = end + 1;
        starts.push(pos as u32);
        if !ascii {
            cum.push(chars);
        }
    }
    let tail = &chunk[pos..];
    if tail.contains(&b'\r') {
        return None;
    }
    chars += if ascii { tail.len() as u32 } else { count_chars(tail) };
    if !last {
        // the line that starts at the chunk's end belongs to the next chunk
        starts.pop();
        cum.pop();
    }
    Some(Chunk { starts, cum, chars, ascii, breaks, crlfs })
}

// the char index of each line start in an ASCII chunk, from its line starts: one char per byte, less the "\r" of each
// earlier "\r\n"
fn ascii_cum(chunk: &[u8], starts: &[u32]) -> Vec<u32> {
    let mut crs = 0u32;
    starts
        .iter()
        .enumerate()
        .map(|(j, &s)| {
            if j > 0 && s >= 2 && chunk[s as usize - 2] == b'\r' {
                crs += 1;
            }
            s - crs
        })
        .collect()
}

fn is_fixed_drive(path: &Path) -> bool {
    use windows::core::PCWSTR;
    let Ok(full) = std::fs::canonicalize(path) else { return false };
    let full = full.to_string_lossy().into_owned();
    let full = full.strip_prefix(r"\\?\").unwrap_or(&full);
    let b = full.as_bytes();
    if b.len() < 3 || b[1] != b':' {
        return false;
    }
    let root = crate::sys::wide(&format!("{}:\\", b[0] as char));
    unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_FIXED }
}

impl Mapped {
    /// None when the file should go through the normal loader: too small or too large, not on a local fixed drive, a
    /// cloud placeholder, UTF-16, not valid UTF-8, or containing a lone '\r'.
    pub fn open(path: &Path) -> Option<Mapped> {
        Self::open_min(path, MAP_MIN_BYTES)
    }

    pub fn open_min(path: &Path, min_bytes: u64) -> Option<Mapped> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let meta = std::fs::metadata(path).ok()?;
        if meta.len() < min_bytes || meta.len() > MAP_MAX_BYTES || meta.len() == 0 {
            return None;
        }
        if meta.file_attributes() & UNSAFE_TO_MAP != 0 || !is_fixed_drive(path) {
            return None;
        }
        let file = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path).ok()?;
        // Safe because the sharing mode above keeps every other process from writing, truncating or deleting the file.
        let map = unsafe { Mmap::map(&file).ok()? };
        let bytes: &[u8] = &map;
        let skip = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
            return None;
        }
        let data = &bytes[skip..];
        let cuts = crate::doc::chunk_cuts(data);
        let parts: Vec<Option<Chunk>> = std::thread::scope(|scope| {
            let handles: Vec<_> = cuts
                .windows(2)
                .enumerate()
                .map(|(i, w)| {
                    let (chunk, last) = (&data[w[0]..w[1]], i + 2 == cuts.len());
                    scope.spawn(move || index_chunk(chunk, last))
                })
                .collect();
            handles.into_iter().map(|h| h.join().ok().flatten()).collect()
        });
        let parts: Vec<Chunk> = parts.into_iter().collect::<Option<_>>()?;
        let (breaks, crlfs) = parts.iter().fold((0, 0), |(b, c), p| (b + p.breaks, c + p.crlfs));
        let formula = parts.iter().all(|p| p.ascii) && (crlfs == 0 || crlfs == breaks);
        let mut starts = Vec::with_capacity(breaks as usize + 2);
        let mut cum = Vec::with_capacity(if formula { 0 } else { breaks as usize + 2 });
        let mut chars = 0u32;
        for (part, w) in parts.into_iter().zip(cuts.windows(2)) {
            if !formula {
                let local = if part.ascii { ascii_cum(&data[w[0]..w[1]], &part.starts) } else { part.cum };
                cum.extend(local.iter().map(|c| c + chars));
            }
            starts.extend(part.starts.iter().map(|s| s + w[0] as u32));
            chars += part.chars;
        }
        starts.push(data.len() as u32);
        let crlf = starts.len() > 2 && starts[1] >= 2 && data[starts[1] as usize - 2] == b'\r';
        let chars = if formula {
            CharIndex::Ascii { crlf: crlfs > 0 }
        } else {
            cum.push(chars);
            CharIndex::Table(cum)
        };
        Some(Mapped { map, _file: file, skip, starts, chars, crlf })
    }

    // char index of line `i`'s start; `i == lines()` gives the total
    fn char_of_line(&self, i: usize) -> usize {
        match &self.chars {
            CharIndex::Table(cum) => cum[i] as usize,
            // every line but the last ends in a break
            CharIndex::Ascii { crlf } => self.starts[i] as usize - *crlf as usize * i.min(self.lines() - 1),
        }
    }

    pub fn has_bom(&self) -> bool {
        self.skip > 0
    }

    pub fn is_crlf(&self) -> bool {
        self.crlf
    }

    fn data(&self) -> &[u8] {
        &self.map[self.skip..]
    }

    fn str_range(&self, a: usize, b: usize) -> &str {
        // Valid UTF-8 was verified when the file was indexed, and every offset used here is a line boundary or a char
        // boundary inside a line.
        unsafe { std::str::from_utf8_unchecked(&self.data()[a..b]) }
    }

    pub fn lines(&self) -> usize {
        self.starts.len() - 1
    }

    pub fn total_chars(&self) -> usize {
        self.char_of_line(self.lines())
    }

    pub fn char_to_line(&self, idx: usize) -> usize {
        let idx = idx.min(self.total_chars());
        // the last line whose start is at or before idx
        let (mut lo, mut hi) = (0, self.lines());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.char_of_line(mid) <= idx {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1)
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.char_of_line(line.min(self.lines() - 1))
    }

    pub fn line_len(&self, line: usize) -> usize {
        let n = self.lines();
        if line + 1 < n {
            self.char_of_line(line + 1) - self.char_of_line(line) - 1
        } else {
            self.char_of_line(n) - self.char_of_line(line.min(n - 1))
        }
    }

    // byte range of a line without its line break
    fn line_bytes(&self, line: usize) -> (usize, usize) {
        let n = self.lines();
        let start = self.starts[line] as usize;
        if line + 1 >= n {
            return (start, self.starts[n] as usize);
        }
        let mut end = self.starts[line + 1] as usize - 1;
        if end > start && self.data()[end - 1] == b'\r' {
            end -= 1;
        }
        (start, end)
    }

    pub fn line_str(&self, line: usize) -> &str {
        let (s, e) = self.line_bytes(line.min(self.lines() - 1));
        self.str_range(s, e)
    }

    fn byte_of_char(&self, idx: usize) -> usize {
        if idx >= self.total_chars() {
            return self.data().len();
        }
        let line = self.char_to_line(idx);
        let col = idx - self.char_of_line(line);
        let (s, e) = self.line_bytes(line);
        match self.str_range(s, e).char_indices().nth(col) {
            Some((b, _)) => s + b,
            None => e,
        }
    }

    fn str_from(&self, idx: usize) -> &str {
        self.str_range(self.byte_of_char(idx), self.data().len())
    }

    fn str_before(&self, idx: usize) -> &str {
        self.str_range(0, self.byte_of_char(idx))
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        let (ba, bb) = (self.byte_of_char(a), self.byte_of_char(b));
        self.str_range(ba, bb.max(ba)).replace('\r', "")
    }

    pub fn to_rope(&self) -> Rope {
        crate::doc::rope_from_utf8(self.data()).unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
