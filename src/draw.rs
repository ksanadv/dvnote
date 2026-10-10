// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::editor::{LineLayout, Metrics, Piece};
use crate::utf16::Utf16Map;
use std::any::Any;
use std::rc::Rc;
use windows::core::{w, Result, HRESULT, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_UNKNOWN;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_HIT_TEST_METRICS,
    DWRITE_LINE_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_READING_DIRECTION_RIGHT_TO_LEFT,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows_numerics::Vector2;

#[derive(Clone, Copy)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn d2d(self, a: f32) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0 as f32 / 255.0, g: self.1 as f32 / 255.0, b: self.2 as f32 / 255.0, a }
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: Rgb,
    pub fg: Rgb,
    pub selection: Rgb,
    pub selection_inactive: Rgb,
    pub status_bg: Rgb,
    pub status_fg: Rgb,
    pub link: Rgb,
    pub code_bg: Rgb,
    pub rule: Rgb,
    pub muted: Rgb,
    // hovered or open menu bar item
    pub hot: Rgb,
}

impl Theme {
    pub fn light() -> Self {
        Theme {
            dark: false,
            bg: Rgb(255, 255, 255),
            fg: Rgb(32, 32, 32),
            selection: Rgb(173, 214, 255),
            selection_inactive: Rgb(225, 225, 225),
            status_bg: Rgb(245, 245, 245),
            status_fg: Rgb(64, 64, 64),
            link: Rgb(0, 102, 204),
            code_bg: Rgb(243, 243, 243),
            rule: Rgb(224, 224, 224),
            muted: Rgb(110, 110, 110),
            hot: Rgb(225, 225, 225),
        }
    }

    pub fn dark() -> Self {
        Theme {
            dark: true,
            bg: Rgb(30, 30, 30),
            fg: Rgb(212, 212, 212),
            selection: Rgb(38, 79, 120),
            selection_inactive: Rgb(58, 61, 65),
            status_bg: Rgb(37, 37, 38),
            status_fg: Rgb(204, 204, 204),
            link: Rgb(86, 156, 214),
            code_bg: Rgb(40, 40, 40),
            rule: Rgb(62, 62, 66),
            muted: Rgb(140, 140, 140),
            hot: Rgb(62, 62, 66),
        }
    }

    pub fn system() -> Self {
        use winreg::enums::HKEY_CURRENT_USER;
        let light: u32 = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_value("AppsUseLightTheme"))
            .unwrap_or(1);
        if light == 0 {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    pub fn for_mode(mode: crate::settings::ThemeMode) -> Self {
        match mode {
            crate::settings::ThemeMode::System => Theme::system(),
            crate::settings::ThemeMode::Light => Theme::light(),
            crate::settings::ThemeMode::Dark => Theme::dark(),
        }
    }
}

const E_NOT_SUFFICIENT_BUFFER: HRESULT = HRESULT(0x8007007Au32 as i32);

pub struct Gfx {
    pub dwrite: IDWriteFactory,
    pub d2d: ID2D1Factory,
}

impl Gfx {
    pub fn new() -> Result<Rc<Gfx>> {
        unsafe {
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            Ok(Rc::new(Gfx { dwrite, d2d }))
        }
    }

    pub fn text_format(&self, family: &str, size_px: f32) -> Result<IDWriteTextFormat> {
        let name = crate::sys::wide(family);
        unsafe {
            self.dwrite.CreateTextFormat(
                PCWSTR(name.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size_px,
                w!(""),
            )
        }
    }

    pub fn layout(&self, text: &[u16], format: &IDWriteTextFormat, width: f32) -> Result<IDWriteTextLayout> {
        unsafe { self.dwrite.CreateTextLayout(text, format, width, 1.0e7) }
    }
}

// Hebrew, Arabic and related blocks
fn is_rtl_char(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

fn starts_rtl(text: &str) -> bool {
    text.chars().find(|c| c.is_alphabetic()).is_some_and(is_rtl_char)
}

pub struct DwLayout {
    gfx: Rc<Gfx>,
    format: IDWriteTextFormat,
    metrics: Metrics,
}

impl DwLayout {
    pub fn new(gfx: Rc<Gfx>, family: &str, size_px: f32, scale: f32) -> Result<Self> {
        let format = gfx.text_format(family, size_px)?;
        let probe = gfx.layout(&[b'0' as u16], &format, 1.0e4)?;
        let (mut tm, mut lines, mut n) = (DWRITE_TEXT_METRICS::default(), [DWRITE_LINE_METRICS::default()], 0u32);
        unsafe {
            probe.GetMetrics(&mut tm)?;
            probe.GetLineMetrics(Some(&mut lines), &mut n)?;
            format.SetLineSpacing(
                DWRITE_LINE_SPACING_METHOD_UNIFORM,
                lines[0].height.ceil(),
                lines[0].baseline.ceil(),
            )?;
            format.SetIncrementalTabStop(tm.width * 4.0)?;
        }
        let metrics = Metrics {
            row_h: lines[0].height.ceil(),
            char_w: tm.width,
            margin_x: (12.0 * scale).round(),
            margin_y: (6.0 * scale).round(),
        };
        Ok(DwLayout { gfx, format, metrics })
    }
}

impl LineLayout for DwLayout {
    fn metrics(&self) -> Metrics {
        self.metrics
    }

    fn layout(&mut self, text: &str, wrap_width: f32) -> Rc<dyn Piece> {
        Rc::new(DwPiece::new(&self.gfx, &self.format, text, wrap_width, self.metrics.row_h))
    }
}

pub struct DwPiece {
    pub layout: IDWriteTextLayout,
    map: Utf16Map,
    rows: u32,
    width: f32,
    row_h: f32,
    starts: Vec<usize>,
    trailing_ws: Vec<usize>,
}

impl DwPiece {
    fn new(gfx: &Gfx, format: &IDWriteTextFormat, text: &str, wrap_width: f32, row_h: f32) -> Self {
        let map = Utf16Map::new(text);
        let wrap = wrap_width.is_finite();
        let layout = gfx.layout(&map.units, format, if wrap { wrap_width } else { 1.0e6 }).expect("text layout");
        let (mut tm, mut count) = (DWRITE_TEXT_METRICS::default(), 0u32);
        let mut lines = Vec::new();
        let rtl = starts_rtl(text);
        unsafe {
            let _ = layout.SetWordWrapping(if wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            if rtl {
                let _ = layout.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT);
            }
            let _ = layout.GetMetrics(&mut tm);
            if !wrap && rtl {
                // without a wrap width an RTL line would sit at the far edge of the huge layout box
                let _ = layout.SetMaxWidth(tm.widthIncludingTrailingWhitespace.max(1.0));
            }
            let _ = layout.GetLineMetrics(None, &mut count);
            lines.resize(count as usize, DWRITE_LINE_METRICS::default());
            let _ = layout.GetLineMetrics(Some(&mut lines), &mut count);
        }
        let (mut starts, mut trailing_ws, mut at) = (Vec::new(), Vec::new(), 0usize);
        for l in &lines {
            starts.push(map.to_char(at));
            trailing_ws.push(
                map.to_char(at + l.length as usize)
                    - map.to_char(at + (l.length - l.trailingWhitespaceLength) as usize),
            );
            at += l.length as usize;
        }
        if starts.is_empty() {
            starts.push(0);
            trailing_ws.push(0);
        }
        DwPiece { layout, map, rows: count.max(1), width: tm.width, row_h, starts, trailing_ws }
    }

    fn row_of(&self, col: usize) -> usize {
        self.starts.partition_point(|&s| s <= col).saturating_sub(1)
    }

    pub fn draw(&self, rt: &ID2D1HwndRenderTarget, brush: &ID2D1SolidColorBrush, x: f32, y: f32) {
        unsafe { rt.DrawTextLayout(Vector2 { X: x, Y: y }, &self.layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE) };
    }
}

impl Piece for DwPiece {
    fn rows(&self) -> u32 {
        self.rows
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn caret(&self, col: usize) -> (f32, f32) {
        let unit = self.map.to_unit(col) as u32;
        let (mut x, mut y, mut m) = (0.0f32, 0.0f32, DWRITE_HIT_TEST_METRICS::default());
        unsafe {
            let _ = self.layout.HitTestTextPosition(unit, false, &mut x, &mut y, &mut m);
        }
        // snap to the row: DirectWrite reports a fraction of a pixel of offset inside the row
        (x, (y / self.row_h).round() * self.row_h)
    }

    fn hit(&self, x: f32, y: f32) -> usize {
        let (mut trailing, mut inside, mut m) =
            (Default::default(), Default::default(), DWRITE_HIT_TEST_METRICS::default());
        unsafe {
            let _ = self.layout.HitTestPoint(x, y.max(0.0), &mut trailing, &mut inside, &mut m);
        }
        let unit = m.textPosition as usize + if trailing.as_bool() { m.length as usize } else { 0 };
        let mut col = self.map.to_char(unit);
        let row = ((y / self.row_h).floor().max(0.0) as usize).min(self.starts.len() - 1);
        if row + 1 < self.starts.len() && col >= self.starts[row + 1] {
            col = self.starts[row + 1] - 1;
        }
        col.min(self.map.char_count())
    }

    fn row_start(&self, col: usize) -> usize {
        self.starts[self.row_of(col)]
    }

    fn row_end(&self, col: usize) -> usize {
        let row = self.row_of(col);
        if row + 1 < self.starts.len() {
            (self.starts[row + 1] - self.trailing_ws[row].max(1)).max(self.starts[row])
        } else {
            self.map.char_count()
        }
    }

    fn range_rects(&self, a: usize, b: usize) -> Vec<[f32; 4]> {
        let (ua, ub) = (self.map.to_unit(a) as u32, self.map.to_unit(b) as u32);
        let mut v = vec![DWRITE_HIT_TEST_METRICS::default(); 16];
        let mut actual = 0u32;
        loop {
            let r = unsafe { self.layout.HitTestTextRange(ua, ub - ua, 0.0, 0.0, Some(&mut v), &mut actual) };
            match r {
                Ok(()) => break,
                Err(e) if e.code() == E_NOT_SUFFICIENT_BUFFER => {
                    v.resize(actual as usize, DWRITE_HIT_TEST_METRICS::default())
                }
                Err(_) => {
                    actual = 0;
                    break;
                }
            }
        }
        v.truncate(actual as usize);
        v.iter().map(|m| [m.left, m.top, m.width, m.height]).collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct Canvas {
    gfx: Rc<Gfx>,
    hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    size: (u32, u32),
}

impl Canvas {
    pub fn new(gfx: Rc<Gfx>, hwnd: HWND) -> Self {
        Canvas { gfx, hwnd, rt: None, brush: None, size: (0, 0) }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        let size = (w.max(1), h.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        if let Some(rt) = &self.rt {
            unsafe {
                let _ = rt.Resize(&D2D_SIZE_U { width: self.size.0, height: self.size.1 });
            }
        }
    }

    fn ensure(&mut self) -> Option<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)> {
        if self.rt.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_UNKNOWN, alphaMode: D2D1_ALPHA_MODE_UNKNOWN },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: Default::default(),
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: self.size.0, height: self.size.1 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = unsafe { self.gfx.d2d.CreateHwndRenderTarget(&props, &hwnd_props).ok()? };
            let brush = unsafe { rt.CreateSolidColorBrush(&Rgb(0, 0, 0).d2d(1.0), None).ok()? };
            unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE) };
            self.rt = Some(rt);
            self.brush = Some(brush);
        }
        Some((self.rt.clone()?, self.brush.clone()?))
    }

    pub fn paint(&mut self, bg: Rgb, f: impl FnOnce(&Pen)) {
        let Some((rt, brush)) = self.ensure() else { return };
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&bg.d2d(1.0)));
        }
        f(&Pen { rt: &rt, brush: &brush });
        let result = unsafe { rt.EndDraw(None, None) };
        if result.is_err() {
            // device lost or target invalid: rebuild on the next paint
            self.rt = None;
            self.brush = None;
        }
    }
}

pub struct Pen<'a> {
    rt: &'a ID2D1HwndRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
}

impl Pen<'_> {
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        unsafe {
            self.brush.SetColor(&color.d2d(1.0));
            self.rt.FillRectangle(&D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, self.brush);
        }
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgb) {
        unsafe {
            self.brush.SetColor(&color.d2d(1.0));
            self.rt.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.brush, width, None);
        }
    }

    pub fn piece(&self, piece: &dyn Piece, x: f32, y: f32, color: Rgb) {
        if let Some(p) = piece.as_any().downcast_ref::<DwPiece>() {
            unsafe { self.brush.SetColor(&color.d2d(1.0)) };
            p.draw(self.rt, self.brush, x, y);
        }
    }

    pub fn layout_links(
        &self,
        layout: &IDWriteTextLayout,
        x: f32,
        y: f32,
        color: Rgb,
        links: &[(u32, u32)],
        link_color: Rgb,
    ) {
        unsafe {
            if !links.is_empty() {
                if let Ok(b) = self.rt.CreateSolidColorBrush(&link_color.d2d(1.0), None) {
                    for (start, len) in links {
                        let _ = layout.SetDrawingEffect(&b, DWRITE_TEXT_RANGE { startPosition: *start, length: *len });
                    }
                }
            }
            self.brush.SetColor(&color.d2d(1.0));
            self.rt.DrawTextLayout(Vector2 { X: x, Y: y }, layout, self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> DwLayout {
        DwLayout::new(Gfx::new().unwrap(), "Consolas", 15.0, 1.0).unwrap()
    }

    #[test]
    fn metrics_are_sane() {
        let m = layout().metrics();
        assert!(m.char_w > 5.0 && m.char_w < 15.0, "{}", m.char_w);
        assert!(m.row_h >= 15.0 && m.row_h < 40.0, "{}", m.row_h);
    }

    #[test]
    fn monospace_caret_and_hit_test_agree() {
        let mut l = layout();
        let m = l.metrics();
        let p = l.layout("hello world", f32::INFINITY);
        assert_eq!(p.rows(), 1);
        for col in 0..=11 {
            let (x, y) = p.caret(col);
            assert!((x - col as f32 * m.char_w).abs() < 0.6, "col {col}: x={x}");
            assert_eq!(y, 0.0);
            assert_eq!(p.hit(x + 0.5, m.row_h / 2.0), col);
        }
        assert_eq!(p.hit(5000.0, 3.0), 11);
        assert_eq!(p.hit(-20.0, 3.0), 0);
    }

    #[test]
    fn emoji_and_cjk_use_char_columns() {
        let mut l = layout();
        let p = l.layout("a🙂b日本c", f32::INFINITY);
        assert_eq!(p.rows(), 1);
        let xs: Vec<f32> = (0..=6).map(|c| p.caret(c).0).collect();
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "carets must advance: {xs:?}");
        for col in 0..=6 {
            assert_eq!(p.hit(xs[col] + 0.5, 3.0), col, "col {col}, xs={xs:?}");
        }
        assert!(p.width() > 0.0);
    }

    #[test]
    fn wrapping_makes_rows_with_uniform_height() {
        let mut l = layout();
        let m = l.metrics();
        let text = "word ".repeat(40);
        let p = l.layout(&text, 20.0 * m.char_w);
        assert!(p.rows() >= 8, "{} rows", p.rows());
        let (_, y0) = p.caret(0);
        let (_, y_last) = p.caret(text.chars().count());
        assert_eq!(y0, 0.0);
        assert!((y_last - (p.rows() - 1) as f32 * m.row_h).abs() < 0.5, "last row y={y_last}");
        // Home/End stay inside their row
        let mid = p.caret(60).1;
        let start = p.row_start(60);
        let end = p.row_end(60);
        assert!(start <= 60 && end >= start);
        assert_eq!(p.caret(start).1, mid);
        assert_eq!(p.caret(end).1, mid);
        // clicking past the end of a wrapped row stays on that row
        let col = p.hit(1000.0, m.row_h * 1.5);
        assert_eq!(p.caret(col).1, m.row_h);
    }

    #[test]
    fn fallback_fonts_keep_the_row_height() {
        let mut l = layout();
        let m = l.metrics();
        for text in ["plain text", "日本語のテキスト", "שלום עולם", "مرحبا بالعالم", "🙂🙂🙂"]
        {
            let p = l.layout(text, f32::INFINITY);
            assert_eq!(p.rows(), 1, "{text}");
            assert_eq!(p.range_rects(0, 1).first().map(|r| r[3]), Some(m.row_h), "{text}");
        }
    }

    #[test]
    fn hebrew_is_laid_out_right_to_left() {
        let mut l = layout();
        let p = l.layout("שלום", f32::INFINITY);
        let (x0, _) = p.caret(0);
        let (x4, _) = p.caret(4);
        assert!(x0 > x4, "caret 0 should be right of caret 4 in RTL text ({x0} vs {x4})");
        let rects = p.range_rects(0, 4);
        assert!(!rects.is_empty());
    }

    #[test]
    fn selection_rects_cover_the_range() {
        let mut l = layout();
        let m = l.metrics();
        let p = l.layout("abcdefghij", f32::INFINITY);
        let r = p.range_rects(2, 5);
        assert_eq!(r.len(), 1);
        assert!((r[0][0] - 2.0 * m.char_w).abs() < 0.6);
        assert!((r[0][2] - 3.0 * m.char_w).abs() < 0.6);
    }

    #[test]
    fn empty_line_has_one_row() {
        let mut l = layout();
        let p = l.layout("", f32::INFINITY);
        assert_eq!(p.rows(), 1);
        assert_eq!(p.caret(0).1, 0.0);
        assert_eq!(p.hit(10.0, 3.0), 0);
        assert_eq!(p.row_start(0), 0);
        assert_eq!(p.row_end(0), 0);
    }
}

// DirectWrite's shared factory caches font data process-wide, so touching the editor and preview fonts on a spare
// thread while the main thread builds its windows makes the first real layout cheaper
pub fn warm_up() {
    let Ok(gfx) = Gfx::new() else { return };
    let units: Vec<u16> = "The quick brown fox 0123".encode_utf16().collect();
    for family in ["Consolas", "Segoe UI"] {
        if let Ok(format) = gfx.text_format(family, 15.0) {
            if let Ok(layout) = gfx.layout(&units, &format, 400.0) {
                let mut m = DWRITE_TEXT_METRICS::default();
                unsafe {
                    let _ = layout.GetMetrics(&mut m);
                }
            }
        }
    }
}
