// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use super::*;
use crate::buffer::Kind;
use crate::text::Text;

const ROW_H: f32 = 16.0;
const CHAR_W: f32 = 8.0;
const MX: f32 = 8.0;
const MY: f32 = 4.0;

struct FakePiece {
    len: usize,
    cols: usize,
}

impl FakePiece {
    fn rows_for(&self) -> usize {
        self.len.div_ceil(self.cols).max(1)
    }
}

impl Piece for FakePiece {
    fn rows(&self) -> u32 {
        self.rows_for() as u32
    }
    fn width(&self) -> f32 {
        self.len.min(self.cols) as f32 * CHAR_W
    }
    fn caret(&self, col: usize) -> (f32, f32) {
        let col = col.min(self.len);
        let (row, c) = if col == self.len && self.len > 0 && self.len % self.cols == 0 {
            (self.len / self.cols - 1, self.cols)
        } else {
            (col / self.cols, col % self.cols)
        };
        (c as f32 * CHAR_W, row as f32 * ROW_H)
    }
    fn hit(&self, x: f32, y: f32) -> usize {
        let rows = self.rows_for();
        let row = ((y / ROW_H).floor().max(0.0) as usize).min(rows - 1);
        let in_row = if row + 1 < rows { self.cols } else { self.len - row * self.cols };
        let max_c = if row + 1 < rows { in_row - 1 } else { in_row };
        let c = ((x / CHAR_W).round().max(0.0) as usize).min(max_c);
        row * self.cols + c
    }
    fn row_start(&self, col: usize) -> usize {
        col.min(self.len) / self.cols * self.cols
    }
    fn row_end(&self, col: usize) -> usize {
        let start = self.row_start(col);
        (start + self.cols - usize::from(start + self.cols < self.len)).min(self.len)
    }
    fn range_rects(&self, _: usize, _: usize) -> Vec<[f32; 4]> {
        Vec::new()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct Fake {
    laid_out: usize,
}

impl LineLayout for Fake {
    fn metrics(&self) -> Metrics {
        Metrics { row_h: ROW_H, char_w: CHAR_W, margin_x: MX, margin_y: MY }
    }
    fn layout(&mut self, text: &str, wrap_width: f32) -> Rc<dyn Piece> {
        self.laid_out += 1;
        let cols =
            if wrap_width.is_finite() { (wrap_width / CHAR_W).floor().max(1.0) as usize } else { usize::MAX / 2 };
        Rc::new(FakePiece { len: text.chars().count(), cols })
    }
}

struct H {
    ed: Editor,
    buf: Buffer,
    lay: Fake,
    wrap: bool,
}

impl H {
    fn new(text: &str) -> Self {
        let mut h = H { ed: Editor::new(), buf: Buffer::from_str(text), lay: Fake { laid_out: 0 }, wrap: false };
        h.cfg();
        h
    }
    fn cfg(&mut self) {
        self.ed.configure(&mut self.lay, &mut self.buf, self.wrap, 600.0, 400.0);
    }
    fn key(&mut self, key: Key) {
        self.cfg();
        self.ed.key(&mut self.lay, &mut self.buf, key, Mods::default());
    }
    fn shift(&mut self, key: Key) {
        self.cfg();
        self.ed.key(&mut self.lay, &mut self.buf, key, Mods { shift: true, ctrl: false });
    }
    fn ctrl(&mut self, key: Key) {
        self.cfg();
        self.ed.key(&mut self.lay, &mut self.buf, key, Mods { shift: false, ctrl: true });
    }
    fn type_(&mut self, s: &str) {
        self.cfg();
        self.ed.type_text(&mut self.buf, s, Kind::Typing);
    }
    fn text(&self) -> String {
        self.buf.text.to_string()
    }
    fn down(&mut self, x: f32, y: f32, clicks: u8) {
        self.cfg();
        self.ed.mouse_down(&mut self.lay, &self.buf, x, y, false, clicks);
    }
}

#[test]
fn typing_enter_and_backspace() {
    let mut h = H::new("");
    h.type_("hello");
    h.key(Key::Enter);
    h.type_("world");
    assert_eq!(h.text(), "hello\nworld");
    assert_eq!(h.ed.cursor, 11);
    h.key(Key::Backspace);
    assert_eq!(h.text(), "hello\nworl");
    h.key(Key::Left);
    h.key(Key::Delete);
    assert_eq!(h.text(), "hello\nwor");
}

#[test]
fn selection_replaced_by_typing() {
    let mut h = H::new("hello world");
    h.key(Key::Home);
    for _ in 0..5 {
        h.shift(Key::Right);
    }
    assert_eq!(h.ed.selection(), (0, 5));
    h.type_("bye");
    assert_eq!(h.text(), "bye world");
    assert_eq!(h.ed.cursor, 3);
    h.shift(Key::Right);
    h.key(Key::Left);
    assert_eq!(h.ed.selection(), (3, 3));
}

#[test]
fn home_end_and_document_ends() {
    let mut h = H::new("one\ntwo\nthree");
    h.ctrl(Key::Home);
    assert_eq!(h.ed.cursor, 0);
    h.key(Key::End);
    assert_eq!(h.ed.cursor, 3);
    h.key(Key::Down);
    h.key(Key::Home);
    assert_eq!(h.ed.cursor, 4);
    h.ctrl(Key::End);
    assert_eq!(h.ed.cursor, 13);
}

#[test]
fn up_down_keep_column() {
    let mut h = H::new("abcdef\nab\nabcdef");
    h.ctrl(Key::Home);
    for _ in 0..4 {
        h.key(Key::Right);
    }
    h.key(Key::Down);
    assert_eq!(h.ed.cursor, 9);
    h.key(Key::Down);
    assert_eq!(h.ed.cursor, 10 + 4);
    h.key(Key::Up);
    h.key(Key::Up);
    assert_eq!(h.ed.cursor, 4);
}

#[test]
fn undo_redo() {
    let mut h = H::new("");
    h.type_("abc");
    h.key(Key::Enter);
    h.type_("def");
    assert_eq!(h.text(), "abc\ndef");
    h.ctrl(Key::Z);
    assert_eq!(h.text(), "abc\n");
    h.ctrl(Key::Z);
    h.ctrl(Key::Z);
    assert_eq!(h.text(), "");
    h.ctrl(Key::Y);
    assert_eq!(h.text(), "abc");
    h.cfg();
    h.ed.key(&mut h.lay, &mut h.buf, Key::Z, Mods { shift: true, ctrl: true });
    assert_eq!(h.text(), "abc\n");
}

#[test]
fn select_all_copy_cut_paste() {
    let mut h = H::new("alpha beta");
    h.ctrl(Key::A);
    assert_eq!(h.ed.copy(&h.buf).as_deref(), Some("alpha beta"));
    assert_eq!(h.ed.cut(&mut h.buf).as_deref(), Some("alpha beta"));
    assert_eq!(h.text(), "");
    h.ed.paste(&mut h.buf, "one\r\ntwo");
    assert_eq!(h.text(), "one\ntwo");
    assert!(h.ed.copy(&h.buf).is_none());
}

#[test]
fn word_deletion() {
    let mut h = H::new("foo bar baz");
    h.ctrl(Key::End);
    h.ctrl(Key::Backspace);
    assert_eq!(h.text(), "foo bar ");
    h.ctrl(Key::Backspace);
    assert_eq!(h.text(), "foo ");
}

#[test]
fn click_places_caret_by_geometry() {
    let mut h = H::new("first line\nsecond line\nthird");
    h.down(MX + CHAR_W * 3.6, MY + ROW_H * 1.5, 1);
    assert_eq!(h.ed.cursor, 11 + 4);
    h.down(MX + 5.0, 380.0, 1);
    assert_eq!(h.ed.cursor, h.buf.len_chars());
}

#[test]
fn double_and_triple_click() {
    let mut h = H::new("foo bar_baz\nnext line");
    h.down(MX + CHAR_W * 5.0, MY + 2.0, 2);
    assert_eq!(h.ed.selection(), (4, 11));
    h.down(MX + CHAR_W * 5.0, MY + 2.0, 3);
    assert_eq!(h.ed.selection(), (0, 12));
}

#[test]
fn drag_selects_a_range() {
    let mut h = H::new("abcdefghij");
    let y = MY + ROW_H * 0.5;
    h.down(MX + CHAR_W * 2.2, y, 1);
    h.ed.mouse_drag(&mut h.lay, &h.buf, MX + CHAR_W * 6.2, y);
    assert_eq!(h.ed.selection(), (2, 6));
}

#[test]
fn wrapped_line_moves_by_visual_row() {
    let long = "word ".repeat(60);
    let mut h = H::new(&long);
    h.wrap = true;
    h.ctrl(Key::Home);
    h.key(Key::Down);
    assert!(h.ed.cursor > 20 && h.ed.cursor < long.chars().count());
    assert_eq!(h.buf.line_of(h.ed.cursor), 0);
    h.key(Key::Home);
    let row_start = h.ed.cursor;
    h.key(Key::End);
    assert!(h.ed.cursor > row_start);
}

#[test]
fn long_lines_are_laid_out_in_pieces_but_addressed_exactly() {
    let line: String = (0..30_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let mut h = H::new(&line);
    h.wrap = true;
    h.cfg();
    for idx in [0, 100, SEG - 1, SEG, SEG + 1, 2 * SEG, 29_999, 30_000] {
        let (x, y) = h.ed.caret_content(&mut h.lay, &h.buf, idx);
        let back = h.ed.pos_at(&mut h.lay, &h.buf, x, y + ROW_H * 0.5);
        assert_eq!(back, idx, "caret of {idx} maps back to {idx}");
    }
    h.ed.set_selection(SEG + 10, SEG + 10);
    h.type_("XYZ");
    assert_eq!(h.buf.slice(SEG + 10, SEG + 13), "XYZ");
    assert_eq!(h.buf.len_chars(), 30_003);
}

#[test]
fn only_visible_lines_are_laid_out() {
    let text = "line\n".repeat(100_000);
    let mut h = H::new(&text);
    h.cfg();
    let before = h.lay.laid_out;
    let vis = h.ed.visible(&mut h.lay, &h.buf);
    assert!(vis.len() <= (400.0 / ROW_H) as usize + 2, "{} pieces", vis.len());
    assert!(h.lay.laid_out - before <= vis.len() + 2);
    h.ctrl(Key::End);
    let vis = h.ed.visible(&mut h.lay, &h.buf);
    assert!(h.ed.scroll_y > 1_000_000.0);
    assert!(vis.len() <= 27);
    let (_, cy) = h.ed.caret_pos(&mut h.lay, &h.buf);
    assert!(cy >= 0.0 && cy < 400.0, "caret on screen, got {cy}");
}

#[test]
fn selection_and_line_break_are_reported_for_painting() {
    let mut h = H::new("ab\ncd\nef");
    h.ed.set_selection(1, 4);
    h.cfg();
    let vis = h.ed.visible(&mut h.lay, &h.buf);
    assert_eq!(vis[0].sel, Some((1, 2)));
    assert!(vis[0].eol.is_some());
    assert_eq!(vis[1].sel, Some((0, 1)));
    assert!(vis[1].eol.is_none());
    assert_eq!(vis[2].sel, None);
}

#[test]
fn mouse_wheel_scrolling_is_clamped() {
    let mut h = H::new(&"x\n".repeat(200));
    h.cfg();
    h.ed.scroll_rows(-5.0);
    assert_eq!(h.ed.scroll_y, 0.0);
    h.ed.scroll_rows(10.0);
    assert_eq!(h.ed.scroll_y, 160.0);
    h.ed.scroll_rows(1e9);
    assert!(h.ed.scroll_y <= h.ed.content_height());
}

#[test]
fn word_motion() {
    let r = Text::from_str("foo bar_baz  qux.x");
    assert_eq!(word_right(&r, 0), 4);
    assert_eq!(word_right(&r, 4), 13);
    assert_eq!(word_right(&r, 13), 16);
    assert_eq!(word_left(&r, 18), 17);
    assert_eq!(word_left(&r, 13), 4);
    assert_eq!(word_left(&r, 4), 0);
    assert_eq!(word_left(&r, 0), 0);
    assert_eq!(word_right(&r, 18), 18);
}

#[test]
fn double_click_range() {
    let r = Text::from_str("foo bar_baz\nx");
    assert_eq!(word_range(&r, 5), (4, 11));
    assert_eq!(word_range(&r, 3), (3, 4));
    assert_eq!(word_range(&r, 0), (0, 3));
    assert_eq!(word_range(&Text::empty(), 0), (0, 0));
}

#[test]
fn edits_refresh_only_the_touched_lines_and_never_show_stale_layouts() {
    let mut h = H::new("aa\nbbb\ncccc");
    let widths = |h: &mut H| -> Vec<f32> {
        h.cfg();
        h.ed.visible(&mut h.lay, &h.buf).iter().map(|v| v.piece.width()).collect()
    };
    assert_eq!(widths(&mut h), [16.0, 24.0, 32.0]);

    let laid = h.lay.laid_out;
    h.ed.set_selection(6, 6);
    h.type_("b");
    assert_eq!(widths(&mut h), [16.0, 32.0, 32.0]);
    assert_eq!(h.lay.laid_out - laid, 1, "only the edited line is laid out again");

    // a line break shifts every line below it
    h.ed.set_selection(0, 0);
    h.key(Key::Enter);
    assert_eq!(widths(&mut h), [0.0, 16.0, 32.0, 32.0]);

    // joining the lines again
    h.key(Key::Backspace);
    assert_eq!(widths(&mut h), [16.0, 32.0, 32.0]);
    for _ in 0..3 {
        h.ctrl(Key::Z);
    }
    assert_eq!(widths(&mut h), [16.0, 24.0, 32.0]);
}
