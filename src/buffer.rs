// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::text::Text;
use ropey::Rope;
use std::borrow::Cow;

#[derive(Clone)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
}

impl Edit {
    fn inserted_chars(&self) -> usize {
        self.inserted.chars().count()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Typing,
    Backspace,
    Delete,
    Other,
}

struct Group {
    edits: Vec<Edit>,
    kind: Kind,
    before: (usize, usize),
    after: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDelta {
    pub first: usize,
    pub removed: usize,
    pub added: usize,
}

const MAX_UNDO_GROUPS: usize = 20_000;
const MAX_MERGED_TYPING: usize = 256;

pub struct Buffer {
    pub text: Text,
    undo: Vec<Group>,
    redo: Vec<Group>,
    pub generation: u64,
    deltas: Vec<LineDelta>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::from_rope(Rope::new())
    }
}

impl Buffer {
    pub fn from_rope(rope: Rope) -> Self {
        Buffer::from_text(Text::Rope(rope))
    }

    pub fn from_text(text: Text) -> Self {
        Buffer { text, undo: Vec::new(), redo: Vec::new(), generation: 0, deltas: Vec::new() }
    }

    #[cfg(test)]
    pub fn from_str(s: &str) -> Self {
        Buffer::from_text(Text::from_str(s))
    }

    pub fn reset(&mut self, text: Text) {
        self.text = text;
        self.undo.clear();
        self.redo.clear();
        self.deltas.clear();
        self.generation += 1;
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    pub fn line_of(&self, idx: usize) -> usize {
        self.text.char_to_line(idx)
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.text.line_to_char(line)
    }

    pub fn line_len(&self, line: usize) -> usize {
        self.text.line_len(line)
    }

    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        self.text.line_text(line)
    }

    pub fn line_col(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.len_chars());
        let line = self.text.char_to_line(idx);
        (line + 1, idx - self.text.line_to_char(line) + 1)
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        self.text.slice(a, b)
    }

    // writing needs a rope; a mapped file is converted (and released) here
    pub fn rope_for_save(&mut self) -> &Rope {
        self.text.rope_mut()
    }

    pub fn take_deltas(&mut self) -> Vec<LineDelta> {
        std::mem::take(&mut self.deltas)
    }

    fn raw_replace(&mut self, at: usize, remove_len: usize, insert: &str) {
        let rope = self.text.rope_mut();
        if remove_len > 0 {
            let (la, lb) = (rope.char_to_line(at), rope.char_to_line(at + remove_len));
            rope.remove(at..at + remove_len);
            self.deltas.push(LineDelta { first: la, removed: lb - la + 1, added: 1 });
        }
        if !insert.is_empty() {
            let line = rope.char_to_line(at);
            rope.insert(at, insert);
            let newlines = insert.bytes().filter(|&b| b == b'\n').count();
            self.deltas.push(LineDelta { first: line, removed: 1, added: 1 + newlines });
        }
        self.generation += 1;
    }

    fn apply(&mut self, e: &Edit) {
        self.raw_replace(e.at, e.removed.chars().count(), &e.inserted);
    }

    fn unapply(&mut self, e: &Edit) {
        self.raw_replace(e.at, e.inserted_chars(), &e.removed);
    }

    pub fn replace(
        &mut self,
        a: usize,
        b: usize,
        text: &str,
        kind: Kind,
        before: (usize, usize),
        after: (usize, usize),
    ) {
        let edit = Edit { at: a, removed: self.slice(a, b), inserted: text.to_owned() };
        if edit.removed.is_empty() && edit.inserted.is_empty() {
            return;
        }
        self.apply(&edit);
        self.redo.clear();
        if self.try_merge(&edit, kind, before, after) {
            return;
        }
        self.push_group(Group { edits: vec![edit], kind, before, after });
    }

    pub fn replace_many(&mut self, edits: &[(usize, usize, String)], before: (usize, usize), after: (usize, usize)) {
        if edits.is_empty() {
            return;
        }
        let mut applied = Vec::with_capacity(edits.len());
        for (a, b, text) in edits.iter().rev() {
            let edit = Edit { at: *a, removed: self.slice(*a, *b), inserted: text.clone() };
            self.apply(&edit);
            applied.push(edit);
        }
        self.redo.clear();
        self.push_group(Group { edits: applied, kind: Kind::Other, before, after });
    }

    fn push_group(&mut self, g: Group) {
        self.undo.push(g);
        if self.undo.len() > MAX_UNDO_GROUPS {
            self.undo.drain(..MAX_UNDO_GROUPS / 10);
        }
    }

    fn try_merge(&mut self, edit: &Edit, kind: Kind, before: (usize, usize), after: (usize, usize)) -> bool {
        let Some(last) = self.undo.last_mut() else { return false };
        if last.kind != kind || last.after != before || last.edits.len() != 1 {
            return false;
        }
        let prev = &mut last.edits[0];
        let merged = match kind {
            Kind::Typing => {
                let ok = edit.removed.is_empty()
                    && prev.removed.is_empty()
                    && prev.at + prev.inserted_chars() == edit.at
                    && !edit.inserted.contains('\n')
                    && prev.inserted.len() < MAX_MERGED_TYPING;
                if ok {
                    prev.inserted.push_str(&edit.inserted);
                }
                ok
            }
            Kind::Backspace => {
                let ok = edit.inserted.is_empty()
                    && prev.inserted.is_empty()
                    && edit.at + edit.removed.chars().count() == prev.at;
                if ok {
                    prev.at = edit.at;
                    prev.removed = format!("{}{}", edit.removed, prev.removed);
                }
                ok
            }
            Kind::Delete => {
                let ok = edit.inserted.is_empty() && prev.inserted.is_empty() && edit.at == prev.at;
                if ok {
                    prev.removed.push_str(&edit.removed);
                }
                ok
            }
            Kind::Other => false,
        };
        if merged {
            last.after = after;
        }
        merged
    }

    pub fn undo(&mut self) -> Option<(usize, usize)> {
        let g = self.undo.pop()?;
        for e in g.edits.iter().rev() {
            self.unapply(e);
        }
        let restore = g.before;
        self.redo.push(g);
        Some(restore)
    }

    pub fn redo(&mut self) -> Option<(usize, usize)> {
        let g = self.redo.pop()?;
        for e in &g.edits {
            self.apply(e);
        }
        let restore = g.after;
        self.undo.push(g);
        Some(restore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &Buffer) -> String {
        b.text.to_string()
    }

    #[test]
    fn lines_only_break_on_lf() {
        let b = Buffer::from_str("a\u{2028}b\rc\nd");
        assert_eq!(b.len_lines(), 2);
        assert_eq!(b.line_text(0), "a\u{2028}b\rc");
        assert_eq!(b.line_len(1), 1);
    }

    #[test]
    fn line_col_and_len() {
        let b = Buffer::from_str("one\ntwo\nthree");
        assert_eq!(b.line_col(0), (1, 1));
        assert_eq!(b.line_col(5), (2, 2));
        assert_eq!(b.line_col(13), (3, 6));
        assert_eq!(b.line_start(2), 8);
        let b = Buffer::from_str("a\n");
        assert_eq!(b.len_lines(), 2);
        assert_eq!(b.line_text(1), "");
    }

    #[test]
    fn typing_merges_and_undoes_in_one_step() {
        let mut b = Buffer::default();
        for (i, c) in "hello".chars().enumerate() {
            b.replace(i, i, &c.to_string(), Kind::Typing, (i, i), (i + 1, i + 1));
        }
        assert_eq!(text(&b), "hello");
        assert_eq!(b.undo(), Some((0, 0)));
        assert_eq!(text(&b), "");
        assert_eq!(b.redo(), Some((5, 5)));
        assert_eq!(text(&b), "hello");
        assert_eq!(b.undo.len(), 1);
    }

    #[test]
    fn newline_breaks_typing_group() {
        let mut b = Buffer::default();
        b.replace(0, 0, "a", Kind::Typing, (0, 0), (1, 1));
        b.replace(1, 1, "\n", Kind::Typing, (1, 1), (2, 2));
        b.replace(2, 2, "b", Kind::Typing, (2, 2), (3, 3));
        assert_eq!(b.undo.len(), 2);
    }

    #[test]
    fn backspace_and_delete_merge() {
        let mut b = Buffer::from_str("abcdef");
        b.replace(5, 6, "", Kind::Backspace, (6, 6), (5, 5));
        b.replace(4, 5, "", Kind::Backspace, (5, 5), (4, 4));
        assert_eq!(text(&b), "abcd");
        assert_eq!(b.undo.len(), 1);
        b.undo();
        assert_eq!(text(&b), "abcdef");

        let mut b = Buffer::from_str("abcdef");
        b.replace(1, 2, "", Kind::Delete, (1, 1), (1, 1));
        b.replace(1, 2, "", Kind::Delete, (1, 1), (1, 1));
        assert_eq!(text(&b), "adef");
        assert_eq!(b.undo.len(), 1);
        b.undo();
        assert_eq!(text(&b), "abcdef");
    }

    #[test]
    fn replace_many_is_one_step() {
        let mut b = Buffer::from_str("a-b-c");
        b.replace_many(&[(1, 2, "+".into()), (3, 4, "++".into())], (0, 0), (0, 0));
        assert_eq!(text(&b), "a+b++c");
        b.undo();
        assert_eq!(text(&b), "a-b-c");
        b.redo();
        assert_eq!(text(&b), "a+b++c");
    }

    #[test]
    fn edit_after_undo_clears_redo() {
        let mut b = Buffer::from_str("x");
        b.replace(1, 1, "y", Kind::Other, (1, 1), (2, 2));
        b.undo();
        b.replace(1, 1, "z", Kind::Other, (1, 1), (2, 2));
        assert!(b.redo().is_none());
        assert_eq!(text(&b), "xz");
    }

    #[test]
    fn deltas_describe_line_changes() {
        let mut b = Buffer::from_str("one\ntwo\nthree");
        b.take_deltas();
        b.replace(4, 4, "x\ny\n", Kind::Other, (0, 0), (0, 0));
        assert_eq!(b.take_deltas(), vec![LineDelta { first: 1, removed: 1, added: 3 }]);
        b.replace(0, 6, "", Kind::Other, (0, 0), (0, 0));
        assert_eq!(b.take_deltas(), vec![LineDelta { first: 0, removed: 3, added: 1 }]);
        assert_eq!(b.len_lines(), text(&b).matches('\n').count() + 1);
    }

    #[test]
    fn unicode_indices_are_chars() {
        let mut b = Buffer::from_str("héllo wörld");
        b.replace(6, 11, "日本", Kind::Other, (0, 0), (0, 0));
        assert_eq!(text(&b), "héllo 日本");
        b.undo();
        assert_eq!(text(&b), "héllo wörld");
    }
}
