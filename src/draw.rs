// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::editor::{LineLayout, Metrics, Piece};
use crate::utf16::Utf16Map;
use std::any::Any;
use std::ffi::c_void;
use std::rc::Rc;
use windows::core::{implement, w, IUnknown, Interface, Ref, Result, BOOL, HRESULT, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteBitmapRenderTarget, IDWriteFactory, IDWriteGdiInterop, IDWriteInlineObject,
    IDWritePixelSnapping_Impl, IDWriteRenderingParams, IDWriteTextFormat, IDWriteTextLayout, IDWriteTextRenderer,
    IDWriteTextRenderer_Impl, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_RUN, DWRITE_GLYPH_RUN_DESCRIPTION, DWRITE_HIT_TEST_METRICS,
    DWRITE_LINE_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_MATRIX, DWRITE_MEASURING_MODE,
    DWRITE_READING_DIRECTION, DWRITE_READING_DIRECTION_RIGHT_TO_LEFT, DWRITE_STRIKETHROUGH, DWRITE_TEXT_METRICS,
    DWRITE_TEXT_RANGE, DWRITE_UNDERLINE, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, GdiFlush, GetCurrentObject, GetDC, GetObjectW, ReleaseDC, SetPixelV, DIBSECTION, HDC, OBJ_BITMAP, SRCCOPY,
};

#[derive(Clone, Copy)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    // the layout of a 32-bit DIB pixel: blue in the low byte
    fn pixel(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }

    fn colorref(self) -> COLORREF {
        COLORREF(self.0 as u32 | (self.1 as u32) << 8 | (self.2 as u32) << 16)
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

// DirectWrite only: it shapes text and rasterizes glyphs into our own bitmaps (see Canvas). There is no Direct2D, so
// no Direct3D device, WARP rasterizer or swap buffers are created.
pub struct Gfx {
    pub dwrite: IDWriteFactory,
    interop: IDWriteGdiInterop,
    // ClearType and gamma as the user configured them
    params: IDWriteRenderingParams,
    renderer: IDWriteTextRenderer,
}

impl Gfx {
    pub fn new() -> Result<Rc<Gfx>> {
        unsafe {
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let interop = dwrite.GetGdiInterop()?;
            let params = dwrite.CreateRenderingParams()?;
            Ok(Rc::new(Gfx { dwrite, interop, params, renderer: GlyphSink.into() }))
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

// A window's back buffer: a 32-bit DIB that DirectWrite draws glyphs into and that rectangles are written into directly,
// then copied to the window in one BitBlt. It is the only pixel memory the editor holds.
struct Surface {
    target: IDWriteBitmapRenderTarget,
    dc: HDC,
    bits: *mut u32,
    w: usize,
    h: usize,
    // pixels per row
    stride: usize,
    top_down: bool,
}

impl Surface {
    fn new(gfx: &Gfx, w: u32, h: u32) -> Option<Surface> {
        let target = unsafe { gfx.interop.CreateBitmapRenderTarget(None, w, h).ok()? };
        // it defaults to the screen's DPI, but layouts are already sized in pixels
        unsafe { target.SetPixelsPerDip(1.0).ok()? };
        let mut s = Surface {
            dc: unsafe { target.GetMemoryDC() },
            target,
            bits: std::ptr::null_mut(),
            w: 0,
            h: 0,
            stride: 0,
            top_down: true,
        };
        s.locate().then_some(s)
    }

    fn resize(&mut self, w: u32, h: u32) -> bool {
        unsafe { self.target.Resize(w, h).is_ok() && self.locate() }
    }

    // finds the DIB's pixels; DirectWrite replaces the bitmap on every resize
    fn locate(&mut self) -> bool {
        unsafe {
            self.dc = self.target.GetMemoryDC();
            let bitmap = GetCurrentObject(self.dc, OBJ_BITMAP);
            let mut ds = DIBSECTION::default();
            let n =
                GetObjectW(bitmap, std::mem::size_of::<DIBSECTION>() as i32, Some(&mut ds as *mut _ as *mut c_void));
            if n as usize != std::mem::size_of::<DIBSECTION>() || ds.dsBm.bmBits.is_null() || ds.dsBm.bmBitsPixel != 32
            {
                return false;
            }
            self.bits = ds.dsBm.bmBits as *mut u32;
            self.w = ds.dsBm.bmWidth as usize;
            self.h = ds.dsBm.bmHeight.unsigned_abs() as usize;
            self.stride = ds.dsBm.bmWidthBytes as usize / 4;
            // the header does not reliably report the row order, so ask GDI to set the top-left pixel and look
            let probe = Rgb(1, 2, 3);
            let _ = SetPixelV(self.dc, 0, 0, probe.colorref());
            let _ = GdiFlush();
            self.top_down = *self.bits & 0xFF_FFFF == probe.pixel();
        }
        true
    }

    fn row(&self, y: usize) -> &mut [u32] {
        let y = if self.top_down { y } else { self.h - 1 - y };
        // SAFETY: y < h and the DIB holds h rows of `stride` pixels; the surface is only touched on the UI thread
        unsafe { std::slice::from_raw_parts_mut(self.bits.add(y * self.stride), self.w) }
    }

    fn clear(&self, c: Rgb) {
        let p = c.pixel();
        for y in 0..self.h {
            self.row(y).fill(p);
        }
    }

    // an axis-aligned rectangle with antialiased edges: partly covered edge pixels are blended by their coverage
    fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
        let (x0, x1) = (x.max(0.0), (x + w).min(self.w as f32));
        let (y0, y1) = (y.max(0.0), (y + h).min(self.h as f32));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (p, solid) = (c.pixel(), |a: f32, b: f32, i: usize| (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0));
        let (cx0, cx1) = (x0.floor() as usize, (x1.ceil() as usize).min(self.w));
        // whole pixels between the partly covered edge columns
        let (ix0, ix1) = (x0.ceil() as usize, (x1.floor() as usize).max(x0.ceil() as usize));
        for py in y0.floor() as usize..(y1.ceil() as usize).min(self.h) {
            let cy = solid(y0, y1, py);
            let row = self.row(py);
            if cy >= 0.999 && ix1 > ix0 {
                row[ix0..ix1].fill(p);
                for px in (cx0..ix0).chain(ix1..cx1) {
                    blend(&mut row[px], p, solid(x0, x1, px));
                }
            } else {
                for px in cx0..cx1 {
                    blend(&mut row[px], p, cy * solid(x0, x1, px));
                }
            }
        }
    }
}

fn blend(dst: &mut u32, src: u32, a: f32) {
    if a >= 0.999 {
        *dst = src;
        return;
    }
    let a = (a * 256.0) as u32;
    let mix = |shift: u32| {
        let (d, s) = ((*dst >> shift) & 0xFF, (src >> shift) & 0xFF);
        ((d * (256 - a) + s * a) >> 8) << shift
    };
    *dst = mix(16) | mix(8) | mix(0);
}

pub struct Canvas {
    gfx: Rc<Gfx>,
    hwnd: HWND,
    surface: Option<Surface>,
    size: (u32, u32),
}

impl Canvas {
    pub fn new(gfx: Rc<Gfx>, hwnd: HWND) -> Self {
        Canvas { gfx, hwnd, surface: None, size: (0, 0) }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        let size = (w.max(1), h.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        if let Some(s) = &mut self.surface {
            if !s.resize(size.0, size.1) {
                self.surface = None;
            }
        }
    }

    // frees the back buffer, for a pane that is hidden
    pub fn release(&mut self) {
        self.surface = None;
    }

    pub fn paint(&mut self, bg: Rgb, f: impl FnOnce(&Pen)) {
        if self.surface.is_none() {
            self.surface = Surface::new(&self.gfx, self.size.0.max(1), self.size.1.max(1));
        }
        let Some(s) = &self.surface else { return };
        s.clear(bg);
        f(&Pen { gfx: &self.gfx, surface: s });
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let _ = BitBlt(dc, 0, 0, s.w as i32, s.h as i32, Some(s.dc), 0, 0, SRCCOPY);
            ReleaseDC(Some(self.hwnd), dc);
        }
    }
}

pub struct Pen<'a> {
    gfx: &'a Gfx,
    surface: &'a Surface,
}

// what a layout's Draw call hands to the glyph sink
struct DrawContext<'a> {
    surface: &'a Surface,
    params: &'a IDWriteRenderingParams,
    color: Rgb,
    link: Rgb,
}

impl Pen<'_> {
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        self.surface.fill(x, y, w, h, color);
    }

    // only horizontal and vertical lines are drawn
    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgb) {
        let half = width / 2.0;
        if x0 == x1 {
            self.surface.fill(x0 - half, y0.min(y1), width, (y1 - y0).abs(), color);
        } else {
            self.surface.fill(x0.min(x1), y0 - half, (x1 - x0).abs(), width, color);
        }
    }

    pub fn piece(&self, piece: &dyn Piece, x: f32, y: f32, color: Rgb) {
        if let Some(p) = piece.as_any().downcast_ref::<DwPiece>() {
            self.text(&p.layout, x, y, color, color);
        }
    }

    // links are marked with a drawing effect; any non-null effect means "draw in the link color"
    pub fn layout_links(
        &self,
        layout: &IDWriteTextLayout,
        x: f32,
        y: f32,
        color: Rgb,
        links: &[(u32, u32)],
        link_color: Rgb,
    ) {
        if !links.is_empty() {
            let marker: IUnknown = self.gfx.params.cast().expect("IUnknown");
            for (start, len) in links {
                let _ = unsafe {
                    layout.SetDrawingEffect(&marker, DWRITE_TEXT_RANGE { startPosition: *start, length: *len })
                };
            }
        }
        self.text(layout, x, y, color, link_color);
    }

    fn text(&self, layout: &IDWriteTextLayout, x: f32, y: f32, color: Rgb, link: Rgb) {
        let ctx = DrawContext { surface: self.surface, params: &self.gfx.params, color, link };
        let _ = unsafe { layout.Draw(Some(&ctx as *const DrawContext as *const c_void), &self.gfx.renderer, x, y) };
    }
}

// receives the glyph runs and decorations of a layout and puts them on the surface in the DrawContext
#[implement(IDWriteTextRenderer)]
struct GlyphSink;

unsafe fn context<'a>(p: *const c_void) -> &'a DrawContext<'a> {
    &*(p as *const DrawContext)
}

fn decoration(
    ctx: &DrawContext,
    x: f32,
    y: f32,
    width: f32,
    offset: f32,
    thick: f32,
    dir: DWRITE_READING_DIRECTION,
    effect: bool,
) {
    let x = if dir == DWRITE_READING_DIRECTION_RIGHT_TO_LEFT { x - width } else { x };
    ctx.surface.fill(x, y + offset, width, thick.max(1.0), if effect { ctx.link } else { ctx.color });
}

impl IDWritePixelSnapping_Impl for GlyphSink_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const c_void) -> Result<BOOL> {
        Ok(false.into())
    }

    fn GetCurrentTransform(&self, _: *const c_void, transform: *mut DWRITE_MATRIX) -> Result<()> {
        unsafe { *transform = DWRITE_MATRIX { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 } };
        Ok(())
    }

    // layouts are already sized in pixels
    fn GetPixelsPerDip(&self, _: *const c_void) -> Result<f32> {
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for GlyphSink_Impl {
    fn DrawGlyphRun(
        &self,
        ctx: *const c_void,
        x: f32,
        y: f32,
        mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        _: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        let ctx = unsafe { context(ctx) };
        let color = if effect.is_null() { ctx.color } else { ctx.link };
        unsafe { ctx.surface.target.DrawGlyphRun(x, y, mode, run, ctx.params, color.colorref(), None) }
    }

    fn DrawUnderline(
        &self,
        ctx: *const c_void,
        x: f32,
        y: f32,
        u: *const DWRITE_UNDERLINE,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        let (ctx, u) = unsafe { (context(ctx), &*u) };
        decoration(ctx, x, y, u.width, u.offset, u.thickness, u.readingDirection, !effect.is_null());
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        ctx: *const c_void,
        x: f32,
        y: f32,
        s: *const DWRITE_STRIKETHROUGH,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        let (ctx, s) = unsafe { (context(ctx), &*s) };
        decoration(ctx, x, y, s.width, s.offset, s.thickness, s.readingDirection, !effect.is_null());
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: Ref<IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
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
