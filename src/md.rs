// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    Code,
    Item,
    Rule,
    TableRow { header: bool },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub text: String,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub body: Text,
    pub marker: Option<String>,
    pub list_depth: u32,
    pub quote_depth: u32,
    pub cells: Vec<Text>,
}

impl Block {
    fn new(kind: BlockKind) -> Self {
        Block { kind, body: Text::default(), marker: None, list_depth: 0, quote_depth: 0, cells: Vec::new() }
    }
}

struct Builder {
    blocks: Vec<Block>,
    cur: Option<Block>,
    cell: Option<Text>,
    row_cells: Vec<Text>,
    in_header: bool,
    styles: Vec<Style>,
    lists: Vec<Option<u64>>,
    quote: u32,
    pending_marker: Option<String>,
    in_code: bool,
}

impl Builder {
    fn style(&self) -> Style {
        let mut s = Style::default();
        for x in &self.styles {
            s.bold |= x.bold;
            s.italic |= x.italic;
            s.code |= x.code;
            s.strike |= x.strike;
            if x.link.is_some() {
                s.link = x.link.clone();
            }
        }
        s
    }

    fn target(&mut self) -> Option<&mut Text> {
        if let Some(c) = self.cell.as_mut() {
            return Some(c);
        }
        self.cur.as_mut().map(|b| &mut b.body)
    }

    fn push_text(&mut self, text: &str) {
        let style = self.style();
        if self.target().is_none() {
            self.start(BlockKind::Paragraph);
        }
        let Some(t) = self.target() else { return };
        let start = t.text.chars().count();
        t.text.push_str(text);
        let end = start + text.chars().count();
        if end > start && (style != Style::default()) {
            t.spans.push(Span { start, end, style });
        }
    }

    fn start(&mut self, kind: BlockKind) {
        self.finish();
        let mut b = Block::new(kind);
        b.list_depth = self.lists.len() as u32;
        b.quote_depth = self.quote;
        self.cur = Some(b);
    }

    fn finish(&mut self) {
        if let Some(b) = self.cur.take() {
            let keep = !b.body.text.is_empty() || matches!(b.kind, BlockKind::Rule | BlockKind::Item | BlockKind::Code);
            if keep {
                self.blocks.push(b);
            }
        }
    }
}

pub fn parse(source: &str) -> Vec<Block> {
    let o = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder {
        blocks: Vec::new(),
        cur: None,
        cell: None,
        row_cells: Vec::new(),
        in_header: false,
        styles: Vec::new(),
        lists: Vec::new(),
        quote: 0,
        pending_marker: None,
        in_code: false,
    };
    for ev in Parser::new_ext(source, o) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => b.start(BlockKind::Heading(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                })),
                Tag::Paragraph => {
                    // a paragraph directly inside a list item continues that item's block
                    let in_item = b.cur.as_ref().is_some_and(|c| c.kind == BlockKind::Item && c.body.text.is_empty());
                    if !in_item {
                        b.start(BlockKind::Paragraph);
                        b.pending_marker = None;
                    }
                }
                Tag::BlockQuote(_) => {
                    b.finish();
                    b.quote += 1;
                }
                Tag::CodeBlock(_) => {
                    b.start(BlockKind::Code);
                    b.in_code = true;
                }
                Tag::List(start) => {
                    b.finish();
                    b.lists.push(start);
                }
                Tag::Item => {
                    b.start(BlockKind::Item);
                    let marker = match b.lists.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}.");
                            *n += 1;
                            m
                        }
                        _ => "\u{2022}".to_owned(),
                    };
                    if let Some(c) = b.cur.as_mut() {
                        c.marker = Some(marker);
                    }
                }
                Tag::Emphasis => b.styles.push(Style { italic: true, ..Default::default() }),
                Tag::Strong => b.styles.push(Style { bold: true, ..Default::default() }),
                Tag::Strikethrough => b.styles.push(Style { strike: true, ..Default::default() }),
                Tag::Link { dest_url, .. } => {
                    b.styles.push(Style { link: Some(dest_url.to_string()), ..Default::default() })
                }
                Tag::Image { .. } => b.push_text("[image: "),
                Tag::Table(_) => b.finish(),
                Tag::TableHead => {
                    b.in_header = true;
                    b.row_cells.clear();
                }
                Tag::TableRow => b.row_cells.clear(),
                Tag::TableCell => b.cell = Some(Text::default()),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) | TagEnd::Paragraph => b.finish(),
                TagEnd::BlockQuote(_) => {
                    b.finish();
                    b.quote = b.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    b.in_code = false;
                    if let Some(c) = b.cur.as_mut() {
                        while c.body.text.ends_with('\n') {
                            c.body.text.pop();
                        }
                    }
                    b.finish();
                }
                TagEnd::List(_) => {
                    b.finish();
                    b.lists.pop();
                }
                TagEnd::Item => b.finish(),
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    b.styles.pop();
                }
                TagEnd::Image => b.push_text("]"),
                TagEnd::TableCell => {
                    if let Some(c) = b.cell.take() {
                        b.row_cells.push(c);
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    let mut blk = Block::new(BlockKind::TableRow { header: b.in_header });
                    blk.cells = std::mem::take(&mut b.row_cells);
                    blk.quote_depth = b.quote;
                    b.blocks.push(blk);
                    if matches!(tag, TagEnd::TableHead) {
                        b.in_header = false;
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if b.in_code {
                    // keep code verbatim: one span-free body
                    if let Some(c) = b.cur.as_mut() {
                        c.body.text.push_str(&t);
                    }
                } else {
                    b.push_text(&t);
                }
            }
            Event::Code(t) => {
                b.styles.push(Style { code: true, ..Default::default() });
                b.push_text(&t);
                b.styles.pop();
            }
            Event::SoftBreak => b.push_text(" "),
            Event::HardBreak => b.push_text("\n"),
            Event::Rule => {
                b.start(BlockKind::Rule);
                b.finish();
            }
            Event::Html(t) | Event::InlineHtml(t) => b.push_text(&t),
            Event::TaskListMarker(done) => b.push_text(if done { "\u{2611} " } else { "\u{2610} " }),
            _ => {}
        }
    }
    b.finish();
    b.blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<BlockKind> {
        parse(src).into_iter().map(|b| b.kind).collect()
    }

    #[test]
    fn headings_and_paragraphs() {
        let b = parse("# Title\n\nSome text.\n\n## Sub\n");
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].kind, BlockKind::Heading(1));
        assert_eq!(b[0].body.text, "Title");
        assert_eq!(b[1].kind, BlockKind::Paragraph);
        assert_eq!(b[2].kind, BlockKind::Heading(2));
    }

    #[test]
    fn inline_styles_become_spans() {
        let b = parse("a **bold** and *it* and `code` and [link](http://x.y) z");
        let t = &b[0].body;
        assert_eq!(t.text, "a bold and it and code and link z");
        let find = |f: &dyn Fn(&Style) -> bool| t.spans.iter().find(|s| f(&s.style)).map(|s| &t.text[s.start..s.end]);
        assert_eq!(find(&|s| s.bold), Some("bold"));
        assert_eq!(find(&|s| s.italic), Some("it"));
        assert_eq!(find(&|s| s.code), Some("code"));
        let link = t.spans.iter().find(|s| s.style.link.is_some()).unwrap();
        assert_eq!(&t.text[link.start..link.end], "link");
        assert_eq!(link.style.link.as_deref(), Some("http://x.y"));
    }

    #[test]
    fn spans_use_char_indices() {
        let b = parse("日本語 **太字** é");
        let s = &b[0].body.spans[0];
        assert_eq!((s.start, s.end), (4, 6));
    }

    #[test]
    fn lists_have_markers_and_depth() {
        let b = parse("- one\n- two\n  - nested\n\n1. first\n2. second\n");
        let items: Vec<_> = b.iter().filter(|x| x.kind == BlockKind::Item).collect();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].marker.as_deref(), Some("\u{2022}"));
        assert_eq!(items[0].body.text, "one");
        assert_eq!(items[2].list_depth, 2);
        assert_eq!(items[3].marker.as_deref(), Some("1."));
        assert_eq!(items[4].marker.as_deref(), Some("2."));
    }

    #[test]
    fn loose_list_items_do_not_double_up() {
        let b = parse("- one\n\n- two\n");
        let items: Vec<_> = b.iter().filter(|x| x.kind == BlockKind::Item).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].body.text, "two");
        assert!(b.iter().all(|x| x.kind == BlockKind::Item));
    }

    #[test]
    fn code_block_is_verbatim() {
        let b = parse("```rust\nfn main() {\n    let x = 1;\n}\n```\n");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].kind, BlockKind::Code);
        assert_eq!(b[0].body.text, "fn main() {\n    let x = 1;\n}");
    }

    #[test]
    fn quote_depth_and_rule() {
        let b = parse("> quoted\n\n---\n\nafter\n");
        assert_eq!(b[0].quote_depth, 1);
        assert_eq!(b[1].kind, BlockKind::Rule);
        assert_eq!(b[2].quote_depth, 0);
    }

    #[test]
    fn tables_become_rows_of_cells() {
        let b = parse("| a | b |\n|---|---|\n| 1 | **2** |\n");
        assert_eq!(
            kinds("| a | b |\n|---|---|\n| 1 | 2 |\n"),
            vec![BlockKind::TableRow { header: true }, BlockKind::TableRow { header: false }]
        );
        assert_eq!(b[0].cells.len(), 2);
        assert_eq!(b[0].cells[1].text, "b");
        assert_eq!(b[1].cells[1].text, "2");
        assert!(b[1].cells[1].spans[0].style.bold);
    }

    #[test]
    fn task_lists_and_breaks() {
        let b = parse("- [x] done\n- [ ] todo\n\nline one\nline two  \nline three\n");
        assert!(b[0].body.text.starts_with('\u{2611}'));
        assert!(b[1].body.text.starts_with('\u{2610}'));
        assert_eq!(b[2].body.text, "line one line two\nline three");
    }

    #[test]
    fn empty_and_plain_documents() {
        assert!(parse("").is_empty());
        assert_eq!(parse("just text").len(), 1);
    }
}
