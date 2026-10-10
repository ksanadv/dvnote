// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::buffer::{Buffer, Kind};
use crate::rowindex::RowIndex;
use crate::text::Text;
use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

// lines longer than this are laid out in pieces of this size
pub const SEG: usize = 8192;

// above this size, wrapped line heights start as the average instead of a per-line estimate
const UNIFORM_ESTIMATE_CHARS: usize = 4_000_000;

#[derive(Clone, Copy, PartialEq)]
pub struct Metrics {
    pub row_h: f32,
    pub char_w: f32,
    pub margin_x: f32,
    pub margin_y: f32,
}

pub trait Piece {
    fn rows(&self) -> u32;
    fn width(&self) -> f32;
    fn caret(&self, col: usize) -> (f32, f32);
    fn hit(&self, x: f32, y: f32) -> usize;
    fn row_start(&self, col: usize) -> usize;
    fn row_end(&self, col: usize) -> usize;
    fn range_rects(&self, a: usize, b: usize) -> Vec<[f32; 4]>;
    fn as_any(&self) -> &dyn Any;
}

pub trait LineLayout {
    fn metrics(&self) -> Metrics;
    fn layout(&mut self, text: &str, wrap_width: f32) -> Rc<dyn Piece>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Tab,
    A,
    Z,
    Y,
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

pub struct Visible {
    pub piece: Rc<dyn Piece>,
    pub x: f32,
    pub y: f32,
    pub sel: Option<(usize, usize)>,
    pub eol: Option<(f32, f32)>,
}

fn segment_count(len: usize) -> usize {
    len.div_ceil(SEG).max(1)
}

// a caret at the end of a line whose length is a multiple of SEG stays at the end of the last segment
fn locate(len: usize, col: usize) -> (usize, usize) {
    if col == len && len > 0 && len % SEG == 0 {
        (len / SEG - 1, SEG)
    } else {
        (col / SEG, col % SEG)
    }
}

fn class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        1
    } else if c.is_whitespace() {
        0
    } else {
        2
    }
}

fn word_left(text: &Text, idx: usize) -> usize {
    let (mut i, mut cls) = (idx, None);
    for c in text.chars_before(idx) {
        let k = class(c);
        match cls {
            None if k == 0 => {}
            None => cls = Some(k),
            Some(want) if k != want => break,
            _ => {}
        }
        i -= 1;
    }
    i
}

fn word_right(text: &Text, idx: usize) -> usize {
    let (mut i, mut cls, mut in_space) = (idx, None, false);
    for c in text.chars_from(idx) {
        let k = class(c);
        if in_space {
            if k != 0 {
                break;
            }
        } else {
            match cls {
                None if k == 0 => in_space = true,
                None => cls = Some(k),
                Some(want) if k != want => {
                    if k == 0 {
                        in_space = true;
                    } else {
                        break;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    i
}

fn word_range(text: &Text, idx: usize) -> (usize, usize) {
    let len = text.len_chars();
    if len == 0 {
        return (0, 0);
    }
    let pivot = if idx >= len { len - 1 } else { idx };
    let k = class(text.char_at(pivot));
    let mut a = pivot;
    while a > 0 && class(text.char_at(a - 1)) == k && text.char_at(a - 1) != '\n' {
        a -= 1;
    }
    let mut b = pivot;
    while b < len && class(text.char_at(b)) == k && text.char_at(b) != '\n' {
        b += 1;
    }
    (a, b)
}

pub struct Editor {
    pub cursor: usize,
    pub anchor: usize,
    want_x: Option<f32>,
    rows: RowIndex,
    rows_key: Option<(bool, u32)>,
    cols: u32,
    max_width: f32,
    cache: HashMap<(usize, usize), Rc<dyn Piece>>,
    seg_rows: HashMap<usize, Vec<u32>>,
    m: Metrics,
    wrap: bool,
    wrap_width: f32,
    view: (f32, f32),
    pub scroll_x: f32,
    pub scroll_y: f32,
    want_visible: bool,
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            cursor: 0,
            anchor: 0,
            want_x: None,
            rows: RowIndex::new(vec![1]),
            rows_key: None,
            cols: 0,
            max_width: 0.0,
            cache: HashMap::new(),
            seg_rows: HashMap::new(),
            m: Metrics { row_h: 18.0, char_w: 8.0, margin_x: 8.0, margin_y: 4.0 },
            wrap: false,
            wrap_width: f32::INFINITY,
            view: (600.0, 400.0),
            scroll_x: 0.0,
            scroll_y: 0.0,
            want_visible: false,
        }
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
        self.anchor = 0;
        self.want_x = None;
        self.rows_key = None;
        self.max_width = 0.0;
        self.cache.clear();
        self.seg_rows.clear();
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
    }

    pub fn invalidate(&mut self) {
        self.rows_key = None;
        self.cache.clear();
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.anchor = anchor;
        self.cursor = cursor;
        self.want_x = None;
        self.want_visible = true;
    }

    fn estimate_piece(&self, n: usize) -> u32 {
        if self.cols == 0 {
            1
        } else {
            (n as u32).div_ceil(self.cols).max(1)
        }
    }

    fn estimate_line(&self, len: usize) -> u32 {
        if len <= SEG {
            return self.estimate_piece(len);
        }
        let (full, rem) = (len / SEG, len % SEG);
        full as u32 * self.estimate_piece(SEG) + if rem > 0 { self.estimate_piece(rem) } else { 0 }
    }

    fn rebuild_rows(&mut self, buf: &Buffer) {
        let n = buf.len_lines();
        // without wrapping every line starts as one row; a long line learns its piece count when first laid out
        let rows = if self.cols == 0 {
            vec![1; n]
        } else if buf.text.is_mapped() || buf.len_chars() > UNIFORM_ESTIMATE_CHARS {
            vec![self.estimate_piece(buf.len_chars() / n); n]
        } else {
            (0..n).map(|i| self.estimate_line(buf.line_len(i))).collect()
        };
        self.rows = RowIndex::new(rows);
        self.cache.clear();
        self.seg_rows.clear();
    }

    fn piece_rows(&mut self, buf: &Buffer, line: usize) -> &mut Vec<u32> {
        if !self.seg_rows.contains_key(&line) {
            let len = buf.line_len(line);
            let v: Vec<u32> = (0..segment_count(len)).map(|s| self.estimate_piece((len - s * SEG).min(SEG))).collect();
            self.seg_rows.insert(line, v);
        }
        self.seg_rows.get_mut(&line).unwrap()
    }

    fn sync_rows(&mut self, buf: &mut Buffer) {
        let deltas = buf.take_deltas();
        if deltas.is_empty() {
            return;
        }
        if deltas.len() > 8 {
            self.rebuild_rows(buf);
            return;
        }
        for d in deltas {
            // only the lines the edit touched lose their layout; when the line count shifts, so does everything below
            let end = if d.removed == d.added { d.first + d.added } else { usize::MAX };
            self.cache.retain(|&(line, _), _| line < d.first || line >= end);
            if d.removed == d.added {
                for i in d.first..(d.first + d.added).min(self.rows.len()) {
                    self.seg_rows.remove(&i);
                    let e = self.estimate_line(buf.line_len(i));
                    self.rows.set(i, e);
                }
            } else {
                // line numbers shift, so per-piece measurements keyed by line are stale
                self.seg_rows.clear();
                let new_rows = (d.first..d.first + d.added).map(|i| self.estimate_line(buf.line_len(i))).collect();
                self.rows.splice(d.first, d.removed, new_rows);
            }
        }
        if self.rows.len() != buf.len_lines() {
            self.rebuild_rows(buf);
        }
    }

    pub fn configure(&mut self, layout: &mut dyn LineLayout, buf: &mut Buffer, wrap: bool, view_w: f32, view_h: f32) {
        let m = layout.metrics();
        let wrap_width = if wrap { (view_w - 2.0 * m.margin_x).max(m.char_w * 8.0) } else { f32::INFINITY };
        let cols = if wrap { (wrap_width / m.char_w).floor().max(1.0) as u32 } else { 0 };
        let changed = m != self.m || wrap != self.wrap;
        self.m = m;
        self.wrap = wrap;
        self.wrap_width = wrap_width;
        self.cols = cols;
        self.view = (view_w, view_h);
        let key = (wrap, cols);
        if self.rows_key != Some(key) || self.rows.len() != buf.len_lines() || changed {
            self.rows_key = Some(key);
            buf.take_deltas();
            self.rebuild_rows(buf);
            if !wrap {
                self.max_width = 0.0;
            }
        } else {
            self.sync_rows(buf);
        }
        let len = buf.len_chars();
        self.cursor = self.cursor.min(len);
        self.anchor = self.anchor.min(len);
        if self.cache.len() > 512 {
            self.cache.clear();
        }
        self.clamp_scroll();
    }

    fn line_top(&self, line: usize) -> f32 {
        self.m.margin_y + self.rows.prefix(line) as f32 * self.m.row_h
    }

    fn piece(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, line: usize, seg: usize) -> Rc<dyn Piece> {
        if let Some(p) = self.cache.get(&(line, seg)) {
            return p.clone();
        }
        let len = buf.line_len(line);
        let (a, b) = (seg * SEG, ((seg + 1) * SEG).min(len));
        let text: Cow<str> = if len <= SEG {
            buf.line_text(line)
        } else {
            let start = buf.line_start(line);
            Cow::Owned(buf.slice(start + a, start + b))
        };
        let p = layout.layout(&text, self.wrap_width);
        let measured = p.rows().max(1);
        if len <= SEG {
            if self.rows.get(line) != measured {
                self.rows.set(line, measured);
            }
        } else {
            let pieces = self.piece_rows(buf, line);
            pieces[seg] = measured;
            let total = pieces.iter().sum();
            if self.rows.get(line) != total {
                self.rows.set(line, total);
            }
        }
        self.max_width = self.max_width.max(p.width());
        self.cache.insert((line, seg), p.clone());
        p
    }

    fn rows_before_piece(&mut self, buf: &Buffer, line: usize, seg: usize) -> u64 {
        if seg == 0 {
            return 0;
        }
        self.piece_rows(buf, line)[..seg].iter().map(|&r| r as u64).sum()
    }

    fn caret_content(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, idx: usize) -> (f32, f32) {
        let line = buf.line_of(idx);
        let col = idx - buf.line_start(line);
        let (seg, col_in) = locate(buf.line_len(line), col);
        let p = self.piece(layout, buf, line, seg);
        let piece_top = self.line_top(line) + self.rows_before_piece(buf, line, seg) as f32 * self.m.row_h;
        let (cx, cy) = p.caret(col_in);
        (self.m.margin_x + cx, piece_top + cy)
    }

    pub fn caret_pos(&mut self, layout: &mut dyn LineLayout, buf: &Buffer) -> (f32, f32) {
        let (x, y) = self.caret_content(layout, buf, self.cursor);
        (x - self.scroll_x, y - self.scroll_y)
    }

    fn pos_at(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, x: f32, y: f32) -> usize {
        let y = (y - self.m.margin_y).max(0.0);
        let row = (y / self.m.row_h) as u64;
        if row >= self.rows.total() {
            return buf.len_chars();
        }
        let line = self.rows.line_at_row(row);
        let len = buf.line_len(line);
        let mut local_row = row - self.rows.prefix(line);
        let mut seg = 0;
        if len > SEG {
            let pieces = self.piece_rows(buf, line).clone();
            while seg + 1 < pieces.len() && local_row >= pieces[seg] as u64 {
                local_row -= pieces[seg] as u64;
                seg += 1;
            }
        }
        let p = self.piece(layout, buf, line, seg);
        let piece_top = self.rows_before_piece(buf, line, seg) as f32 * self.m.row_h;
        let line_top = self.rows.prefix(line) as f32 * self.m.row_h;
        let col = p.hit(x - self.m.margin_x, y - line_top - piece_top);
        let piece_len = (len - seg * SEG).min(SEG);
        buf.line_start(line) + seg * SEG + col.min(piece_len)
    }

    pub fn content_height(&self) -> f32 {
        self.m.margin_y * 2.0 + self.rows.total() as f32 * self.m.row_h
    }

    pub fn content_width(&self) -> f32 {
        if self.wrap {
            self.view.0
        } else {
            (self.max_width + 2.0 * self.m.margin_x + self.m.char_w * 2.0).max(self.view.0)
        }
    }

    fn clamp_scroll(&mut self) {
        let max_y = (self.content_height() - self.view.1).max(0.0);
        let max_x = (self.content_width() - self.view.0).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
        self.scroll_x = if self.wrap { 0.0 } else { self.scroll_x.clamp(0.0, max_x) };
    }

    pub fn scroll_to(&mut self, x: f32, y: f32) {
        self.scroll_x = x;
        self.scroll_y = y;
        self.clamp_scroll();
    }

    pub fn scroll_rows(&mut self, rows: f32) {
        self.scroll_to(self.scroll_x, self.scroll_y + rows * self.m.row_h);
    }

    fn ensure_caret_visible(&mut self, layout: &mut dyn LineLayout, buf: &Buffer) {
        let (x, y) = self.caret_content(layout, buf, self.cursor);
        let (rh, cw) = (self.m.row_h, self.m.char_w);
        if y < self.scroll_y {
            self.scroll_y = y - rh * 0.5;
        } else if y + rh > self.scroll_y + self.view.1 {
            self.scroll_y = y + rh * 1.5 - self.view.1;
        }
        if !self.wrap {
            if x < self.scroll_x + cw * 2.0 {
                self.scroll_x = x - cw * 4.0;
            } else if x > self.scroll_x + self.view.0 - cw * 2.0 {
                self.scroll_x = x - self.view.0 + cw * 4.0;
            }
        }
        self.clamp_scroll();
    }

    pub fn visible(&mut self, layout: &mut dyn LineLayout, buf: &Buffer) -> Vec<Visible> {
        if self.want_visible {
            self.want_visible = false;
            self.ensure_caret_visible(layout, buf);
        }
        let (sel_a, sel_b) = self.selection();
        let n_lines = buf.len_lines();
        let (top, bottom) = (self.scroll_y, self.scroll_y + self.view.1);
        let first_row = ((top - self.m.margin_y).max(0.0) / self.m.row_h) as u64;
        let mut line = self.rows.line_at_row(first_row);
        let mut line_start = buf.line_start(line);
        let mut y = self.line_top(line);
        let mut out = Vec::new();
        while y < bottom && line < n_lines {
            let len = buf.line_len(line);
            let pieces = segment_count(len);
            let mut piece_y = y;
            for seg in 0..pieces {
                let estimated = if pieces == 1 { self.rows.get(line) } else { self.piece_rows(buf, line)[seg] } as f32
                    * self.m.row_h;
                if piece_y + estimated < top {
                    piece_y += estimated;
                    continue;
                }
                if piece_y >= bottom {
                    break;
                }
                let p = self.piece(layout, buf, line, seg);
                let height = p.rows().max(1) as f32 * self.m.row_h;
                let base = line_start + seg * SEG;
                let piece_len = (len - seg * SEG).min(SEG);
                let (mut sel, mut eol) = (None, None);
                if sel_a != sel_b && sel_a <= base + piece_len && sel_b > base {
                    let a = sel_a.max(base) - base;
                    let b = sel_b.min(base + piece_len) - base;
                    if b > a {
                        sel = Some((a, b));
                    }
                    if seg + 1 == pieces && sel_b > base + piece_len && line + 1 < n_lines {
                        eol = Some(p.caret(piece_len));
                    }
                }
                out.push(Visible {
                    piece: p,
                    x: self.m.margin_x - self.scroll_x,
                    y: piece_y - self.scroll_y,
                    sel,
                    eol,
                });
                piece_y += height;
            }
            y += self.rows.get(line) as f32 * self.m.row_h;
            line_start += len + 1;
            line += 1;
        }
        out
    }

    fn move_to(&mut self, idx: usize, extend: bool) {
        self.cursor = idx;
        if !extend {
            self.anchor = idx;
        }
        self.want_x = None;
        self.want_visible = true;
    }

    fn move_vertical(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, rows: i64, extend: bool) {
        let (cx, cy) = self.caret_content(layout, buf, self.cursor);
        let x = self.want_x.unwrap_or(cx);
        let y = cy + self.m.row_h * 0.5 + rows as f32 * self.m.row_h;
        let idx = if y < self.m.margin_y { 0 } else { self.pos_at(layout, buf, x, y) };
        self.move_to(idx, extend);
        self.want_x = Some(x);
    }

    fn delete_selection_or(
        &mut self,
        buf: &mut Buffer,
        range: impl FnOnce(&Self, &Buffer) -> (usize, usize),
        kind: Kind,
    ) {
        let (a, b) = if self.has_selection() { self.selection() } else { range(self, buf) };
        if a == b {
            return;
        }
        let kind = if self.has_selection() { Kind::Other } else { kind };
        buf.replace(a, b, "", kind, (self.cursor, self.anchor), (a, a));
        self.set_selection(a, a);
        self.sync_rows(buf);
    }

    pub fn replace_range(&mut self, buf: &mut Buffer, a: usize, b: usize, text: &str, kind: Kind) {
        let end = a + text.chars().count();
        buf.replace(a, b, text, kind, (self.cursor, self.anchor), (end, end));
        self.set_selection(end, end);
        self.sync_rows(buf);
    }

    pub fn insert_text(&mut self, buf: &mut Buffer, text: &str) {
        let (a, b) = self.selection();
        self.replace_range(buf, a, b, text, Kind::Other);
    }

    pub fn type_text(&mut self, buf: &mut Buffer, text: &str, kind: Kind) {
        if text.is_empty() {
            return;
        }
        let (a, b) = self.selection();
        let kind = if a != b { Kind::Other } else { kind };
        self.replace_range(buf, a, b, text, kind);
    }

    pub fn paste(&mut self, buf: &mut Buffer, text: &str) {
        self.type_text(buf, &crate::doc::normalize(text), Kind::Other);
    }

    pub fn copy(&self, buf: &Buffer) -> Option<String> {
        self.has_selection().then(|| {
            let (a, b) = self.selection();
            buf.slice(a, b)
        })
    }

    pub fn cut(&mut self, buf: &mut Buffer) -> Option<String> {
        let text = self.copy(buf)?;
        self.delete_selection_or(buf, |s, _| (s.cursor, s.cursor), Kind::Other);
        Some(text)
    }

    pub fn undo(&mut self, buf: &mut Buffer, redo: bool) {
        let restored = if redo { buf.redo() } else { buf.undo() };
        if let Some((cursor, anchor)) = restored {
            let len = buf.len_chars();
            self.set_selection(anchor.min(len), cursor.min(len));
            self.sync_rows(buf);
        }
    }

    pub fn select_all(&mut self, buf: &Buffer) {
        self.anchor = 0;
        self.cursor = buf.len_chars();
        self.want_visible = true;
    }

    pub fn key(&mut self, layout: &mut dyn LineLayout, buf: &mut Buffer, key: Key, m: Mods) {
        let (shift, ctrl) = (m.shift, m.ctrl);
        let len = buf.len_chars();
        let (sel_a, sel_b) = self.selection();
        let has_sel = sel_a != sel_b;
        let page = ((self.view.1 / self.m.row_h) as i64 - 1).max(1);
        match key {
            Key::Left => {
                let idx = if ctrl {
                    word_left(&buf.text, self.cursor)
                } else if has_sel && !shift {
                    sel_a
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.move_to(idx, shift);
            }
            Key::Right => {
                let idx = if ctrl {
                    word_right(&buf.text, self.cursor)
                } else if has_sel && !shift {
                    sel_b
                } else {
                    (self.cursor + 1).min(len)
                };
                self.move_to(idx, shift);
            }
            Key::Up => self.move_vertical(layout, buf, -1, shift),
            Key::Down => self.move_vertical(layout, buf, 1, shift),
            Key::PageUp => self.move_vertical(layout, buf, -page, shift),
            Key::PageDown => self.move_vertical(layout, buf, page, shift),
            Key::Home | Key::End => {
                let idx = if ctrl {
                    if key == Key::Home {
                        0
                    } else {
                        len
                    }
                } else {
                    let line = buf.line_of(self.cursor);
                    let col = self.cursor - buf.line_start(line);
                    let (seg, col_in) = locate(buf.line_len(line), col);
                    let p = self.piece(layout, buf, line, seg);
                    let target = if key == Key::Home { p.row_start(col_in) } else { p.row_end(col_in) };
                    buf.line_start(line) + seg * SEG + target
                };
                self.move_to(idx, shift);
            }
            Key::Backspace => {
                if ctrl {
                    self.delete_selection_or(buf, |s, b| (word_left(&b.text, s.cursor), s.cursor), Kind::Other);
                } else {
                    self.delete_selection_or(buf, |s, _| (s.cursor.saturating_sub(1), s.cursor), Kind::Backspace);
                }
            }
            Key::Delete => {
                if ctrl {
                    self.delete_selection_or(buf, |s, b| (s.cursor, word_right(&b.text, s.cursor)), Kind::Other);
                } else {
                    self.delete_selection_or(buf, |s, b| (s.cursor, (s.cursor + 1).min(b.len_chars())), Kind::Delete);
                }
            }
            Key::Enter if !ctrl => self.type_text(buf, "\n", Kind::Other),
            Key::Tab if !shift && !ctrl => self.type_text(buf, "\t", Kind::Typing),
            Key::A if ctrl => self.select_all(buf),
            Key::Z if ctrl => self.undo(buf, shift),
            Key::Y if ctrl => self.undo(buf, true),
            _ => {}
        }
    }

    pub fn mouse_down(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, x: f32, y: f32, shift: bool, clicks: u8) {
        let idx = self.pos_at(layout, buf, x + self.scroll_x, y + self.scroll_y);
        match clicks {
            3 => {
                let line = buf.line_of(idx);
                let a = buf.line_start(line);
                let b = if line + 1 < buf.len_lines() { buf.line_start(line + 1) } else { buf.len_chars() };
                self.set_selection(a, b);
            }
            2 => {
                let (a, b) = word_range(&buf.text, idx);
                self.set_selection(a, b);
            }
            _ => self.move_to(idx, shift),
        }
    }

    pub fn mouse_drag(&mut self, layout: &mut dyn LineLayout, buf: &Buffer, x: f32, y: f32) {
        self.cursor = self.pos_at(layout, buf, x + self.scroll_x, y + self.scroll_y);
        self.want_x = None;
        self.want_visible = true;
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
