// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::draw::{Canvas, Gfx, Pen, Rgb, Theme};
use crate::md::{self, Block, BlockKind, Text};
use crate::rowindex::RowIndex;
use crate::utf16::Utf16Map;
use std::collections::HashMap;
use std::rc::Rc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectWrite::{
    IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_WEIGHT_BOLD, DWRITE_HIT_TEST_METRICS,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE,
};

const HEADING_SCALE: [f32; 6] = [2.0, 1.6, 1.35, 1.15, 1.0, 0.9];

struct Laid {
    // one layout for most blocks; one per cell for table rows
    cells: Vec<Cell>,
    marker: Option<IDWriteTextLayout>,
    height: f32,
}

struct Cell {
    layout: IDWriteTextLayout,
    x: f32,
    width: f32,
    // (start unit, length in units, url)
    links: Vec<(u32, u32, String)>,
}

enum Op {
    Text { layout: IDWriteTextLayout, x: f32, y: f32, color: Rgb, links: Vec<(u32, u32)> },
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgb },
    Line { x0: f32, y0: f32, x1: f32, y1: f32, color: Rgb },
}

pub struct Preview {
    canvas: Canvas,
    gfx: Rc<Gfx>,
    body: IDWriteTextFormat,
    mono: IDWriteTextFormat,
    size: f32,
    scale: f32,
    blocks: Vec<Block>,
    rows: RowIndex,
    laid: HashMap<usize, Laid>,
    view: (f32, f32),
    pub scroll_y: f32,
    pub generation: u64,
    pub too_big: bool,
}

impl Preview {
    pub fn new(gfx: Rc<Gfx>, hwnd: HWND, size: f32, scale: f32) -> windows::core::Result<Self> {
        let body = gfx.text_format("Segoe UI", size)?;
        let mono = gfx.text_format("Consolas", size)?;
        Ok(Preview {
            canvas: Canvas::new(gfx.clone(), hwnd),
            gfx,
            body,
            mono,
            size,
            scale,
            blocks: Vec::new(),
            rows: RowIndex::new(Vec::new()),
            laid: HashMap::new(),
            view: (400.0, 300.0),
            scroll_y: 0.0,
            generation: u64::MAX,
            too_big: false,
        })
    }

    fn margin(&self) -> f32 {
        16.0 * self.scale
    }

    fn gap(&self) -> f32 {
        (10.0 * self.scale).round()
    }

    pub fn set_font(&mut self, size: f32, scale: f32) {
        if let (Ok(b), Ok(m)) = (self.gfx.text_format("Segoe UI", size), self.gfx.text_format("Consolas", size)) {
            self.body = b;
            self.mono = m;
            self.size = size;
            self.scale = scale;
            self.relayout();
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.canvas.resize(w, h);
        let width_changed = (self.view.0 - w as f32).abs() > 0.5;
        self.view = (w as f32, h as f32);
        if width_changed {
            self.relayout();
        }
        self.clamp();
    }

    // drop everything derived from the document while the preview is hidden
    pub fn release(&mut self) {
        if self.blocks.is_empty() && self.laid.is_empty() {
            return;
        }
        self.blocks = Vec::new();
        self.laid = HashMap::new();
        self.rows = RowIndex::new(Vec::new());
        self.generation = u64::MAX;
    }

    pub fn set_blocks(&mut self, source: Option<&str>, generation: u64) {
        self.generation = generation;
        self.too_big = source.is_none();
        self.blocks = source.map(md::parse).unwrap_or_default();
        self.relayout();
    }

    fn content_width(&self) -> f32 {
        (self.view.0 - self.margin() * 2.0).max(40.0)
    }

    fn indent(&self, b: &Block) -> f32 {
        (b.quote_depth as f32 * 18.0 + b.list_depth as f32 * 24.0) * self.scale
    }

    fn estimate(&self, b: &Block) -> u32 {
        let chars_per_row = (self.content_width() / (self.size * 0.55)).max(10.0);
        let rows = (b.body.text.chars().count() as f32 / chars_per_row).ceil().max(1.0);
        (rows * self.size * 1.4 + self.gap()) as u32
    }

    fn relayout(&mut self) {
        self.laid.clear();
        let rows: Vec<u32> = self.blocks.iter().map(|b| self.estimate(b)).collect();
        self.rows = RowIndex::new(rows);
    }

    pub fn content_height(&self) -> f32 {
        self.rows.total() as f32 + self.margin()
    }

    pub fn view_height(&self) -> f32 {
        self.view.1
    }

    fn clamp(&mut self) {
        self.scroll_y = self.scroll_y.clamp(0.0, (self.content_height() - self.view.1).max(0.0));
    }

    pub fn scroll_to(&mut self, y: f32) {
        self.scroll_y = y;
        self.clamp();
    }

    pub fn scroll_by(&mut self, dy: f32) {
        self.scroll_to(self.scroll_y + dy);
    }

    fn apply_spans(&self, layout: &IDWriteTextLayout, text: &Text, map: &Utf16Map) -> Vec<(u32, u32, String)> {
        let mut links = Vec::new();
        for s in &text.spans {
            let (a, b) = (map.to_unit(s.start) as u32, map.to_unit(s.end) as u32);
            let range = DWRITE_TEXT_RANGE { startPosition: a, length: b - a };
            unsafe {
                if s.style.bold {
                    let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range);
                }
                if s.style.italic {
                    let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range);
                }
                if s.style.strike {
                    let _ = layout.SetStrikethrough(true, range);
                }
                if s.style.code {
                    let name = crate::sys::wide("Consolas");
                    let _ = layout.SetFontFamilyName(PCWSTR(name.as_ptr()), range);
                }
                if let Some(url) = &s.style.link {
                    let _ = layout.SetUnderline(true, range);
                    links.push((a, b - a, url.clone()));
                }
            }
        }
        links
    }

    fn make_layout(&self, text: &Text, width: f32, mono: bool, heading: Option<u8>, bold: bool) -> Option<Cell> {
        let map = Utf16Map::new(&text.text);
        let layout = self.gfx.layout(&map.units, if mono { &self.mono } else { &self.body }, width).ok()?;
        let whole = DWRITE_TEXT_RANGE { startPosition: 0, length: map.units.len() as u32 };
        unsafe {
            if let Some(level) = heading {
                let _ = layout.SetFontSize(self.size * HEADING_SCALE[(level as usize - 1).min(5)], whole);
                let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, whole);
            } else if bold {
                let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, whole);
            }
        }
        let links = self.apply_spans(&layout, text, &map);
        Some(Cell { layout, x: 0.0, width, links })
    }

    fn text_height(layout: &IDWriteTextLayout) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = layout.GetMetrics(&mut m);
        }
        m.height
    }

    fn ensure(&mut self, i: usize) {
        if self.laid.contains_key(&i) {
            return;
        }
        let b = &self.blocks[i];
        let cw = self.content_width() - self.indent(b);
        let pad = 8.0 * self.scale;
        let mut cells = Vec::new();
        let mut marker = None;
        let height = match &b.kind {
            BlockKind::Rule => 3.0 * self.scale,
            BlockKind::Code => match self.make_layout(&b.body, cw - pad * 2.0, true, None, false) {
                Some(c) => {
                    let h = Self::text_height(&c.layout) + pad * 2.0;
                    cells.push(c);
                    h
                }
                None => 0.0,
            },
            BlockKind::TableRow { header } => {
                let n = b.cells.len().max(1) as f32;
                let colw = cw / n;
                let mut h = 0.0f32;
                for (k, t) in b.cells.iter().enumerate() {
                    if let Some(mut c) = self.make_layout(t, colw - pad, false, None, *header) {
                        c.x = k as f32 * colw + pad * 0.5;
                        h = h.max(Self::text_height(&c.layout));
                        cells.push(c);
                    }
                }
                h + pad
            }
            kind => {
                let heading = if let BlockKind::Heading(l) = kind { Some(*l) } else { None };
                match self.make_layout(&b.body, cw, false, heading, false) {
                    Some(c) => {
                        let h = Self::text_height(&c.layout);
                        cells.push(c);
                        if let Some(m) = &b.marker {
                            let mt = Text { text: m.clone(), spans: Vec::new() };
                            marker = self.make_layout(&mt, 80.0, false, None, false).map(|c| c.layout);
                        }
                        h
                    }
                    None => 0.0,
                }
            }
        };
        let total = (height + self.gap()).ceil() as u32;
        self.rows.set(i, total);
        self.laid.insert(i, Laid { cells, marker, height });
    }

    pub fn paint(&mut self, theme: &Theme) {
        if self.laid.len() > 400 {
            self.laid.clear();
        }
        self.clamp();
        let (w, h) = self.view;
        let mut ops: Vec<Op> = Vec::new();
        if self.too_big {
            let t = Text {
                text: "The preview is only available for documents up to 1 million characters.".into(),
                spans: Vec::new(),
            };
            if let Some(c) = self.make_layout(&t, w - self.margin() * 2.0, false, None, false) {
                ops.push(Op::Text {
                    layout: c.layout,
                    x: self.margin(),
                    y: self.margin(),
                    color: theme.muted,
                    links: Vec::new(),
                });
            }
        } else if !self.blocks.is_empty() {
            let mut i = self.rows.line_at_row(self.scroll_y as u64);
            let mut y = self.rows.prefix(i) as f32;
            while y < self.scroll_y + h && i < self.blocks.len() {
                self.ensure(i);
                self.ops_for(i, y - self.scroll_y + self.margin() * 0.5, theme, &mut ops);
                y += self.rows.get(i) as f32;
                i += 1;
            }
        }
        self.canvas.paint(theme.bg, |pen| {
            for op in &ops {
                draw_op(pen, op, theme);
            }
        });
    }

    fn ops_for(&self, i: usize, y: f32, theme: &Theme, ops: &mut Vec<Op>) {
        let (b, laid) = (&self.blocks[i], &self.laid[&i]);
        let x0 = self.margin() + self.indent(b);
        let cw = self.content_width() - self.indent(b);
        let pad = 8.0 * self.scale;
        let fg = if b.quote_depth > 0 { theme.muted } else { theme.fg };
        for q in 0..b.quote_depth {
            let bx = self.margin() + (q as f32 * 18.0 + 4.0) * self.scale;
            ops.push(Op::Rect { x: bx, y, w: 3.0 * self.scale, h: laid.height + self.gap() * 0.5, color: theme.rule });
        }
        match &b.kind {
            BlockKind::Rule => ops.push(Op::Line { x0, y0: y + 1.0, x1: x0 + cw, y1: y + 1.0, color: theme.rule }),
            BlockKind::Code => {
                ops.push(Op::Rect { x: x0, y, w: cw, h: laid.height, color: theme.code_bg });
                for c in &laid.cells {
                    ops.push(Op::Text {
                        layout: c.layout.clone(),
                        x: x0 + pad,
                        y: y + pad,
                        color: theme.fg,
                        links: Vec::new(),
                    });
                }
            }
            BlockKind::TableRow { header } => {
                if *header {
                    ops.push(Op::Rect { x: x0, y, w: cw, h: laid.height, color: theme.code_bg });
                }
                let n = b.cells.len().max(1) as f32;
                for k in 0..=b.cells.len() {
                    let lx = x0 + k as f32 * (cw / n);
                    ops.push(Op::Line { x0: lx, y0: y, x1: lx, y1: y + laid.height, color: theme.rule });
                }
                ops.push(Op::Line { x0, y0: y, x1: x0 + cw, y1: y, color: theme.rule });
                ops.push(Op::Line { x0, y0: y + laid.height, x1: x0 + cw, y1: y + laid.height, color: theme.rule });
                for c in &laid.cells {
                    let links = c.links.iter().map(|(a, l, _)| (*a, *l)).collect();
                    ops.push(Op::Text { layout: c.layout.clone(), x: x0 + c.x, y: y + pad * 0.5, color: fg, links });
                }
            }
            _ => {
                if let Some(m) = &laid.marker {
                    ops.push(Op::Text {
                        layout: m.clone(),
                        x: x0 - 20.0 * self.scale,
                        y,
                        color: fg,
                        links: Vec::new(),
                    });
                }
                for c in &laid.cells {
                    let links = c.links.iter().map(|(a, l, _)| (*a, *l)).collect();
                    ops.push(Op::Text { layout: c.layout.clone(), x: x0, y, color: fg, links });
                }
            }
        }
    }

    pub fn link_at(&mut self, x: f32, y: f32) -> Option<String> {
        if self.blocks.is_empty() || self.too_big {
            return None;
        }
        let cy = y + self.scroll_y - self.margin() * 0.5;
        if cy < 0.0 {
            return None;
        }
        let i = self.rows.line_at_row(cy as u64);
        self.ensure(i);
        let local_y = cy - self.rows.prefix(i) as f32;
        let (b, laid) = (&self.blocks[i], &self.laid[&i]);
        let x0 = self.margin() + self.indent(b);
        let pad = 8.0 * self.scale;
        for c in &laid.cells {
            let ox = match b.kind {
                BlockKind::TableRow { .. } => x0 + c.x,
                BlockKind::Code => x0 + pad,
                _ => x0,
            };
            let oy = if matches!(b.kind, BlockKind::Code | BlockKind::TableRow { .. }) { pad * 0.5 } else { 0.0 };
            let (mut trailing, mut inside, mut m) =
                (Default::default(), Default::default(), DWRITE_HIT_TEST_METRICS::default());
            unsafe {
                if c.layout.HitTestPoint(x - ox, local_y - oy, &mut trailing, &mut inside, &mut m).is_err() {
                    continue;
                }
            }
            if !inside.as_bool() || x - ox > c.width {
                continue;
            }
            let pos = m.textPosition;
            if let Some((_, _, url)) = c.links.iter().find(|(a, l, _)| pos >= *a && pos < a + l) {
                return Some(url.clone());
            }
        }
        None
    }
}

fn draw_op(pen: &Pen, op: &Op, theme: &Theme) {
    match op {
        Op::Rect { x, y, w, h, color } => pen.rect(*x, *y, *w, *h, *color),
        Op::Line { x0, y0, x1, y1, color } => pen.line(*x0, *y0, *x1, *y1, 1.0, *color),
        Op::Text { layout, x, y, color, links } => pen.layout_links(layout, *x, *y, *color, links, theme.link),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(md: &str) -> Preview {
        let mut p = Preview::new(Gfx::new().unwrap(), HWND::default(), 15.0, 1.0).unwrap();
        p.resize(500, 400);
        p.set_blocks(Some(md), 1);
        p
    }

    #[test]
    fn blocks_get_real_heights() {
        let mut p = preview(
            "# Title\n\nSome text.\n\n```\ncode\nmore\n```\n\n- a\n- b\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\n---\n",
        );
        let n = p.blocks.len();
        let estimated = p.rows.total();
        for i in 0..n {
            p.ensure(i);
        }
        assert!(n >= 8, "{n} blocks");
        assert!(p.rows.total() > 0);
        assert_ne!(p.rows.total(), estimated, "real heights replace the estimates");
        // headings are taller than body text
        assert!(p.laid[&0].height > p.laid[&1].height);
    }

    #[test]
    fn long_paragraph_wraps_and_narrower_is_taller() {
        let text = "word ".repeat(200);
        let mut p = preview(&text);
        p.ensure(0);
        let wide = p.laid[&0].height;
        p.resize(250, 400);
        p.ensure(0);
        assert!(p.laid[&0].height > wide * 1.5, "{} vs {}", p.laid[&0].height, wide);
    }

    #[test]
    fn link_hit_testing() {
        let mut p = preview("[click here](http://example.com) and plain text");
        let x = p.margin() + 10.0;
        let y = p.margin() * 0.5 + 6.0;
        assert_eq!(p.link_at(x, y).as_deref(), Some("http://example.com"));
        assert_eq!(p.link_at(p.margin() + 350.0, y), None);
        assert_eq!(p.link_at(x, 390.0), None);
    }

    #[test]
    fn scrolling_is_clamped_and_empty_is_safe() {
        let md = (0..200).map(|i| format!("Paragraph number {i}\n\n")).collect::<String>();
        let mut p = preview(&md);
        p.scroll_by(-100.0);
        assert_eq!(p.scroll_y, 0.0);
        p.scroll_by(1e9);
        assert!(p.scroll_y > 0.0 && p.scroll_y <= p.content_height());
        let mut e = preview("");
        assert_eq!(e.link_at(10.0, 10.0), None);
        e.scroll_by(50.0);
        assert_eq!(e.scroll_y, 0.0);
    }

    #[test]
    fn unicode_and_rtl_blocks_lay_out() {
        let mut p = preview("日本語 **太字** 🙂\n\nעברית **מודגש** שלום\n\nمرحبا بالعالم\n");
        for i in 0..p.blocks.len() {
            p.ensure(i);
            assert!(p.laid[&i].height > 0.0);
        }
    }
}
