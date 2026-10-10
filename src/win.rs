// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::assoc;
use crate::buffer::{Buffer, Kind};
use crate::dialogs::{self, FindAction};
use crate::doc::{self, Encoding, LineEnding};
use crate::draw::{Canvas, DwLayout, Gfx, Rgb, Theme};
use crate::editor::{Editor, Key, LineLayout, Mods};
use crate::find;
use crate::preview::Preview;
use crate::settings::{self, Settings, ThemeMode, ViewMode};
use crate::sys::{self, wide, Parent};
use crate::text::Text;
use crate::update;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetWindowDC,
    InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_HIDEPREFIX, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HFONT, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::{SetScrollInfo, SetWindowTheme, ShowScrollBar};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow, CFS_POINT, COMPOSITIONFORM,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_ADD, VK_BACK, VK_CONTROL, VK_DELETE,
    VK_DOWN, VK_END, VK_F3, VK_F5, VK_HOME, VK_LEFT, VK_NEXT, VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SUBTRACT, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, ShellExecuteW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_NEW: u16 = 100;
pub const ID_NEW_WINDOW: u16 = 101;
pub const ID_OPEN: u16 = 102;
pub const ID_SAVE: u16 = 103;
pub const ID_SAVE_AS: u16 = 104;
pub const ID_EXIT: u16 = 105;
pub const ID_UNDO: u16 = 110;
pub const ID_REDO: u16 = 111;
pub const ID_CUT: u16 = 112;
pub const ID_COPY: u16 = 113;
pub const ID_PASTE: u16 = 114;
pub const ID_SELECT_ALL: u16 = 115;
pub const ID_FIND: u16 = 120;
pub const ID_FIND_NEXT: u16 = 121;
pub const ID_FIND_PREV: u16 = 122;
pub const ID_REPLACE: u16 = 123;
pub const ID_GOTO: u16 = 124;
pub const ID_TIME_DATE: u16 = 125;
pub const ID_WRAP: u16 = 130;
pub const ID_VIEW_EDITOR: u16 = 140;
pub const ID_VIEW_SPLIT: u16 = 141;
pub const ID_VIEW_PREVIEW: u16 = 142;
pub const ID_TOGGLE_PREVIEW: u16 = 143;
pub const ID_CYCLE_PREVIEW: u16 = 147;
pub const ID_ZOOM_IN: u16 = 144;
pub const ID_ZOOM_OUT: u16 = 145;
pub const ID_ZOOM_RESET: u16 = 146;
pub const ID_INSTALL: u16 = 150;
pub const ID_UNINSTALL: u16 = 151;
pub const ID_ABOUT: u16 = 152;
pub const ID_CHECK_UPDATES: u16 = 153;
pub const ID_UPDATE_REMINDER: u16 = 154;
pub const ID_THEME_SYSTEM: u16 = 160;
pub const ID_THEME_LIGHT: u16 = 161;
pub const ID_THEME_DARK: u16 = 162;
const ID_ENCODING: u16 = 200;
const ID_LINE_ENDING: u16 = 210;
const ID_UPDATE_DOT: u16 = 220;
// results posted back from the update thread; lparam is a boxed result
const WM_UPDATE_CHECKED: u32 = WM_APP + 1;
const WM_UPDATE_INSTALLED: u32 = WM_APP + 2;

use crate::APP;
const MAX_FILE_BYTES: u64 = 2 << 30;
const BLINK_TIMER: usize = 1;
// 10 half-second blinks, then the caret stays solid and the timer stops
const BLINK_LIMIT: u32 = 10;
const PREVIEW_TIMER: usize = 2;
const PREVIEW_MAX_CHARS: usize = 1_000_000;

pub struct App {
    pub main: HWND,
    pub edit: HWND,
    pub prev: HWND,
    pub preview: Preview,
    preview_pending: bool,
    preview_layout: ViewMode,
    pub gfx: Rc<Gfx>,
    pub layout: DwLayout,
    pub theme: Theme,
    pub scale: f32,
    pub buf: Buffer,
    pub ed: Editor,
    pub path: Option<PathBuf>,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub dirty: bool,
    seen_generation: u64,
    pub settings: Settings,
    canvas: Canvas,
    caret_on: bool,
    blink_ticks: u32,
    blink_running: bool,
    dragging: bool,
    last_dbl: (u32, i32, i32),
    high_surrogate: u16,
    shown_title: String,
    status_font: HFONT,
    status_parts: Vec<(RECT, u16)>,
    // the reminder dot is yellow
    update_due: bool,
    // shown in the status bar while a check or download runs
    update_note: Option<&'static str>,
    pub find_query: String,
    pub find_case: bool,
}

thread_local! {
    static APP_CELL: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP_CELL.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

fn key_down(vk: u16) -> bool {
    unsafe { (GetKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

// reading and decoding does not touch the UI, so startup runs it on a thread while the windows are created
pub fn read_file(path: &Path) -> Result<doc::Loaded, String> {
    if let Ok(m) = std::fs::metadata(path) {
        if m.len() > MAX_FILE_BYTES {
            return Err(format!("{} is too large to open ({} GB limit).", path.display(), MAX_FILE_BYTES >> 30));
        }
    }
    doc::load(path).map_err(|e| format!("Cannot open {}\n\n{e}", path.display()))
}

fn set_scroll(hwnd: HWND, bar: SCROLLBAR_CONSTANTS, max: f32, page: f32, pos: f32) {
    let info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (max as i64).clamp(0, i32::MAX as i64 - 1) as i32,
        nPage: page.max(0.0) as u32,
        nPos: pos as i32,
        nTrackPos: 0,
    };
    unsafe {
        SetScrollInfo(hwnd, bar, &info, true);
    }
}

// where a scrollbar request (arrow, page, thumb drag) wants the view to go
fn scroll_target(hwnd: HWND, bar: SCROLLBAR_CONSTANTS, code: u16, cur: f32, step: f32) -> Option<f32> {
    let mut info =
        SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
    unsafe {
        let _ = GetScrollInfo(hwnd, bar, &mut info);
    }
    let page = info.nPage as f32;
    Some(match SCROLLBAR_COMMAND(code as i32) {
        SB_LINEUP => cur - step,
        SB_LINEDOWN => cur + step,
        SB_PAGEUP => cur - page + step,
        SB_PAGEDOWN => cur + page - step,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos as f32,
        SB_TOP => 0.0,
        SB_BOTTOM => f32::MAX,
        _ => return None,
    })
}

pub fn message(title: &str, text: &str, level: rfd::MessageLevel) {
    if std::env::args().any(|a| a == "--quiet") {
        return;
    }
    let mut d = rfd::MessageDialog::new().set_title(title).set_description(text).set_level(level);
    if let Some(p) = parent() {
        d = d.set_parent(&p);
    }
    d.show();
}

impl App {
    fn new(main: HWND, edit: HWND, prev: HWND, gfx: Rc<Gfx>, settings: Settings) -> windows::core::Result<App> {
        let dpi = unsafe { GetDpiForWindow(main) }.max(96);
        let scale = dpi as f32 / 96.0;
        let layout = DwLayout::new(gfx.clone(), "Consolas", settings.font_size * scale, scale)?;
        let status_font = make_status_font(scale);
        let preview = Preview::new(gfx.clone(), prev, settings.font_size * scale, scale)?;
        let update_due = settings.update_reminder && update::reminder_due();
        Ok(App {
            main,
            edit,
            prev,
            preview,
            preview_pending: false,
            preview_layout: if settings.md_view == ViewMode::Editor { ViewMode::Split } else { settings.md_view },
            canvas: Canvas::new(gfx.clone(), edit),
            gfx,
            layout,
            theme: Theme::for_mode(settings.theme),
            scale,
            buf: Buffer::default(),
            ed: Editor::new(),
            path: None,
            encoding: Encoding::Utf8,
            line_ending: LineEnding::NATIVE,
            dirty: false,
            seen_generation: 0,
            settings,
            caret_on: true,
            blink_ticks: 0,
            blink_running: false,
            dragging: false,
            last_dbl: (0, 0, 0),
            high_surrogate: 0,
            shown_title: String::new(),
            status_font,
            status_parts: Vec::new(),
            update_due,
            update_note: None,
            find_query: String::new(),
            find_case: false,
        })
    }

    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("Untitled".to_owned(), |n| n.to_string_lossy().into_owned())
    }

    fn mark_clean(&mut self) {
        self.dirty = false;
        self.seen_generation = self.buf.generation;
    }

    fn sync_dirty(&mut self) {
        if self.buf.generation != self.seen_generation {
            self.seen_generation = self.buf.generation;
            self.dirty = true;
        }
    }

    // errors are returned so the dialog is shown after the app borrow is released
    pub fn load(&mut self, path: &Path) -> Result<(), String> {
        let loaded = read_file(path)?;
        self.open(path, loaded)
    }

    pub fn open(&mut self, path: &Path, l: doc::Loaded) -> Result<(), String> {
        self.buf.reset(l.text);
        self.ed.reset();
        self.encoding = l.encoding;
        self.line_ending = l.line_ending;
        self.path = Some(path.to_owned());
        self.mark_clean();
        self.apply_doc_view();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::env::set_current_dir(dir);
        }
        self.refresh();
        Ok(())
    }

    fn new_doc(&mut self) {
        self.buf.reset(Text::empty());
        self.ed.reset();
        self.path = None;
        self.apply_doc_view();
        self.encoding = Encoding::Utf8;
        self.line_ending = LineEnding::NATIVE;
        self.mark_clean();
        self.refresh();
    }

    fn write_to(&mut self, path: PathBuf) -> Result<(), String> {
        doc::save(&path, self.buf.rope_for_save(), self.encoding, self.line_ending)
            .map_err(|e| format!("Cannot save {}\n\n{e}", path.display()))?;
        self.path = Some(path);
        self.mark_clean();
        self.refresh();
        Ok(())
    }

    // Markdown files open in the view last used for Markdown; every other file opens as plain text
    fn apply_doc_view(&mut self) {
        let markdown = self.path.as_deref().is_some_and(doc::is_markdown);
        self.settings.view = if markdown { self.settings.md_view } else { ViewMode::Editor };
        self.layout_children();
    }

    pub fn set_view(&mut self, view: ViewMode) {
        self.settings.view = view;
        if view != ViewMode::Editor {
            self.preview_layout = view;
        }
        if self.path.as_deref().is_some_and(doc::is_markdown) {
            self.settings.md_view = view;
            self.settings.save();
        }
        self.layout_children();
    }

    pub fn toggle_preview(&mut self) {
        let target = if self.settings.view == ViewMode::Editor { self.preview_layout } else { ViewMode::Editor };
        self.set_view(target);
    }

    pub fn cycle_preview(&mut self) {
        let target = if self.settings.view == ViewMode::Split { ViewMode::Preview } else { ViewMode::Split };
        self.set_view(target);
    }

    fn restart_blink(&mut self) {
        self.caret_on = true;
        self.blink_ticks = 0;
        let focused = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == self.edit;
        if focused && !self.blink_running {
            self.blink_running = true;
            unsafe {
                SetTimer(Some(self.edit), BLINK_TIMER, GetCaretBlinkTime().max(250), None);
            }
        }
    }

    fn blink_tick(&mut self) {
        self.blink_ticks += 1;
        if self.blink_ticks >= BLINK_LIMIT {
            // idle: leave the caret solid so the window does nothing until the next input
            self.caret_on = true;
            self.blink_running = false;
            unsafe {
                let _ = KillTimer(Some(self.edit), BLINK_TIMER);
            }
            trim_working_set();
        } else {
            self.caret_on = !self.caret_on;
        }
    }

    pub fn apply_theme(&mut self) {
        self.theme = Theme::for_mode(self.settings.theme);
        apply_window_theme(self.main, self.edit, self.prev, &self.theme, self.settings.theme, true);
        self.refresh();
        unsafe {
            let _ = InvalidateRect(Some(self.prev), None, false);
        }
    }

    pub fn zoom(&mut self, factor: f32) {
        self.settings.font_size = (self.settings.font_size * factor).clamp(8.0, 72.0).round();
        self.rebuild_font();
    }

    pub fn reset_zoom(&mut self) {
        self.settings.font_size = Settings::default().font_size;
        self.rebuild_font();
    }

    fn rebuild_font(&mut self) {
        if let Ok(l) = DwLayout::new(self.gfx.clone(), "Consolas", self.settings.font_size * self.scale, self.scale) {
            self.layout = l;
            self.ed.invalidate();
            self.preview.set_font(self.settings.font_size * self.scale, self.scale);
        }
        self.refresh();
    }

    // everything that depends on the document or view changed: repaint and update scrollbars, title and status
    pub fn refresh(&mut self) {
        self.sync_dirty();
        let title = format!("{}{} - {APP}", if self.dirty { "*" } else { "" }, self.display_name());
        if title != self.shown_title {
            unsafe {
                let _ = SetWindowTextW(self.main, PCWSTR(wide(&title).as_ptr()));
            }
            self.shown_title = title;
        }
        self.schedule_preview();
        self.restart_blink();
        unsafe {
            let _ = InvalidateRect(Some(self.edit), None, false);
            let _ = InvalidateRect(Some(self.main), None, false);
        }
    }

    fn update_scrollbars(&self) {
        let (w, h) = client_size(self.edit);
        set_scroll(self.edit, SB_VERT, self.ed.content_height(), h as f32, self.ed.scroll_y);
        if !self.settings.wrap {
            set_scroll(self.edit, SB_HORZ, self.ed.content_width(), w as f32, self.ed.scroll_x);
        }
    }

    fn paint_edit(&mut self) {
        let (w, h) = client_size(self.edit);
        self.canvas.resize(w.max(1) as u32, h.max(1) as u32);
        self.ed.configure(&mut self.layout, &mut self.buf, self.settings.wrap, w as f32, h as f32);
        let vis = self.ed.visible(&mut self.layout, &self.buf);
        let caret = self.ed.caret_pos(&mut self.layout, &self.buf);
        let focused = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == self.edit;
        let (theme, row_h) = (self.theme, self.layout.metrics().row_h);
        let char_w = self.layout.metrics().char_w;
        let caret_on = self.caret_on && focused;
        let sel_color = if focused { theme.selection } else { theme.selection_inactive };
        self.canvas.paint(theme.bg, |pen| {
            for v in &vis {
                if let Some((a, b)) = v.sel {
                    for r in v.piece.range_rects(a, b) {
                        pen.rect(v.x + r[0], v.y + r[1], r[2], r[3].max(row_h), sel_color);
                    }
                }
                if let Some((ex, ey)) = v.eol {
                    pen.rect(v.x + ex, v.y + ey, char_w * 0.6, row_h, sel_color);
                }
                pen.piece(&*v.piece, v.x, v.y, theme.fg);
            }
            if caret_on {
                pen.line(caret.0, caret.1, caret.0, caret.1 + row_h, 1.5, theme.fg);
            }
        });
        self.update_scrollbars();
        self.sync_dirty();
    }

    fn mods() -> Mods {
        Mods { shift: key_down(VK_SHIFT.0), ctrl: key_down(VK_CONTROL.0) }
    }

    fn on_key(&mut self, vk: u16) -> bool {
        let m = Self::mods();
        let key = match vk {
            v if v == VK_LEFT.0 => Key::Left,
            v if v == VK_RIGHT.0 => Key::Right,
            v if v == VK_UP.0 => Key::Up,
            v if v == VK_DOWN.0 => Key::Down,
            v if v == VK_PRIOR.0 => Key::PageUp,
            v if v == VK_NEXT.0 => Key::PageDown,
            v if v == VK_HOME.0 => Key::Home,
            v if v == VK_END.0 => Key::End,
            v if v == VK_BACK.0 => Key::Backspace,
            v if v == VK_DELETE.0 => Key::Delete,
            v if v == VK_RETURN.0 => Key::Enter,
            v if v == VK_TAB.0 => Key::Tab,
            0x41 if m.ctrl => Key::A,
            0x5A if m.ctrl => Key::Z,
            0x59 if m.ctrl => Key::Y,
            0x43 if m.ctrl => return self.copy(false),
            0x58 if m.ctrl => return self.copy(true),
            0x56 if m.ctrl => return self.paste(),
            _ => return false,
        };
        self.ed.key(&mut self.layout, &mut self.buf, key, m);
        self.refresh();
        true
    }

    fn on_char(&mut self, unit: u16) {
        if unit < 0x20 || unit == 0x7F {
            return;
        }
        let text = if (0xD800..0xDC00).contains(&unit) {
            self.high_surrogate = unit;
            return;
        } else if (0xDC00..0xE000).contains(&unit) {
            let hi = std::mem::take(&mut self.high_surrogate);
            String::from_utf16_lossy(&[hi, unit])
        } else {
            String::from_utf16_lossy(&[unit])
        };
        self.ed.type_text(&mut self.buf, &text, Kind::Typing);
        self.refresh();
    }

    pub fn copy(&mut self, cut: bool) -> bool {
        let text = if cut { self.ed.cut(&mut self.buf) } else { self.ed.copy(&self.buf) };
        if let Some(t) = text {
            sys::set_clipboard_text(self.main, &t);
            if cut {
                self.refresh();
            }
        }
        true
    }

    pub fn paste(&mut self) -> bool {
        if let Some(t) = sys::clipboard_text(self.main) {
            self.ed.paste(&mut self.buf, &t);
            self.refresh();
        }
        true
    }

    pub fn insert(&mut self, text: &str) {
        self.ed.insert_text(&mut self.buf, text);
        self.refresh();
    }

    pub fn select_all(&mut self) {
        self.ed.select_all(&self.buf);
        self.refresh();
    }

    pub fn undo(&mut self, redo: bool) {
        self.ed.undo(&mut self.buf, redo);
        self.refresh();
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        self.ed.set_selection(a, b);
        self.refresh();
    }

    pub fn selected_single_line(&self) -> String {
        let (a, b) = self.ed.selection();
        if b - a > 0 && b - a < 200 {
            let s = self.buf.slice(a, b);
            if !s.contains('\n') {
                return s;
            }
        }
        String::new()
    }

    pub fn go_to_line(&mut self, line: usize) {
        let start = self.buf.line_start(line.max(1) - 1);
        self.select_range(start, start);
    }

    pub fn find_next(&mut self, forward: bool) -> bool {
        let (a, b) = self.ed.selection();
        let from = if forward { b } else { a };
        match find::find(&self.buf.text, &self.find_query, from, forward, self.find_case) {
            Some(start) => {
                self.select_range(start, start + self.find_query.chars().count());
                true
            }
            None => false,
        }
    }

    pub fn replace_current(&mut self, replacement: &str) -> bool {
        let (a, b) = self.ed.selection();
        let m = self.find_query.chars().count();
        if m > 0 && b - a == m {
            let selected = self.buf.slice(a, b);
            let same = if self.find_case {
                selected == self.find_query
            } else {
                selected.to_lowercase() == self.find_query.to_lowercase()
            };
            if same {
                self.ed.replace_range(&mut self.buf, a, b, replacement, Kind::Other);
                self.refresh();
            }
        }
        self.find_next(true)
    }

    pub fn replace_all(&mut self, replacement: &str) -> usize {
        let m = self.find_query.chars().count();
        let starts = find::find_all(&self.buf.text, &self.find_query, self.find_case);
        if !starts.is_empty() {
            let edits: Vec<(usize, usize, String)> =
                starts.iter().map(|&s| (s, s + m, replacement.to_owned())).collect();
            let before = (self.ed.cursor, self.ed.anchor);
            self.buf.replace_many(&edits, before, (0, 0));
            self.ed.set_selection(0, 0);
            self.refresh();
        }
        starts.len()
    }

    fn on_mouse_down(&mut self, x: i32, y: i32, double: bool) {
        unsafe {
            let _ = SetFocus(Some(self.edit));
            SetCapture(self.edit);
            let now = GetMessageTime() as u32;
            let dct = GetDoubleClickTime();
            let near = (self.last_dbl.1 - x).abs() < 6 && (self.last_dbl.2 - y).abs() < 6;
            let clicks = if double {
                self.last_dbl = (now, x, y);
                2
            } else if near && now.wrapping_sub(self.last_dbl.0) <= dct {
                self.last_dbl.0 = 0;
                3
            } else {
                1
            };
            let shift = key_down(VK_SHIFT.0);
            self.ed.mouse_down(&mut self.layout, &self.buf, x as f32, y as f32, shift, clicks);
        }
        self.dragging = true;
        self.refresh();
    }

    fn on_mouse_move(&mut self, x: i32, y: i32) {
        if self.dragging {
            self.ed.mouse_drag(&mut self.layout, &self.buf, x as f32, y as f32);
            self.refresh();
        }
    }

    fn on_mouse_up(&mut self) {
        if self.dragging {
            self.dragging = false;
            unsafe {
                let _ = ReleaseCapture();
            }
        }
    }

    fn on_wheel(&mut self, delta: i16) {
        if key_down(VK_CONTROL.0) {
            self.zoom(if delta > 0 { 1.1 } else { 1.0 / 1.1 });
            return;
        }
        let notches = -(delta as f32 / 120.0);
        if key_down(VK_SHIFT.0) {
            let dx = notches * self.layout.metrics().char_w * 6.0;
            self.ed.scroll_to(self.ed.scroll_x + dx, self.ed.scroll_y);
        } else {
            self.ed.scroll_rows(notches * 3.0);
        }
        self.refresh();
    }

    fn on_scroll(&mut self, vertical: bool, code: u16) {
        let m = self.layout.metrics();
        let (bar, step, cur) =
            if vertical { (SB_VERT, m.row_h, self.ed.scroll_y) } else { (SB_HORZ, m.char_w * 4.0, self.ed.scroll_x) };
        let Some(target) = scroll_target(self.edit, bar, code, cur, step) else { return };
        if vertical {
            self.ed.scroll_to(self.ed.scroll_x, target);
        } else {
            self.ed.scroll_to(target, self.ed.scroll_y);
        }
        self.refresh();
    }

    fn place_ime(&mut self) {
        let (cx, cy) = self.ed.caret_pos(&mut self.layout, &self.buf);
        unsafe {
            let himc = ImmGetContext(self.edit);
            if !himc.is_invalid() {
                let form = COMPOSITIONFORM {
                    dwStyle: CFS_POINT,
                    ptCurrentPos: POINT { x: cx as i32, y: cy as i32 },
                    rcArea: RECT::default(),
                };
                let _ = ImmSetCompositionWindow(himc, &form);
                let _ = ImmReleaseContext(self.edit, himc);
            }
        }
    }

    fn status_height(&self) -> i32 {
        (24.0 * self.scale) as i32
    }

    pub fn layout_children(&mut self) {
        let (w, h) = client_size(self.main);
        let ch = (h - self.status_height()).max(0);
        unsafe {
            match self.settings.view {
                ViewMode::Editor => {
                    self.preview.release();
                    let _ = MoveWindow(self.edit, 0, 0, w, ch, true);
                    let _ = ShowWindow(self.edit, SW_SHOW);
                    let _ = ShowWindow(self.prev, SW_HIDE);
                }
                ViewMode::Split => {
                    let half = w / 2;
                    let _ = MoveWindow(self.edit, 0, 0, half - 1, ch, true);
                    let _ = MoveWindow(self.prev, half, 0, w - half, ch, true);
                    let _ = ShowWindow(self.edit, SW_SHOW);
                    let _ = ShowWindow(self.prev, SW_SHOW);
                }
                ViewMode::Preview => {
                    let _ = MoveWindow(self.prev, 0, 0, w, ch, true);
                    let _ = ShowWindow(self.edit, SW_HIDE);
                    let _ = ShowWindow(self.prev, SW_SHOW);
                    let _ = SetFocus(Some(self.prev));
                }
            }
        }
        self.canvas_resize();
        let (pw, ph) = client_size(self.prev);
        self.preview.resize(pw.max(1) as u32, ph.max(1) as u32);
        self.update_preview();
        unsafe {
            let _ = InvalidateRect(Some(self.main), None, false);
            let _ = InvalidateRect(Some(self.prev), None, false);
        }
    }

    fn schedule_preview(&mut self) {
        if self.settings.view != ViewMode::Editor
            && self.preview.generation != self.buf.generation
            && !self.preview_pending
        {
            self.preview_pending = true;
            unsafe {
                SetTimer(Some(self.main), PREVIEW_TIMER, 150, None);
            }
        }
    }

    fn update_preview(&mut self) {
        if self.settings.view == ViewMode::Editor || self.preview.generation == self.buf.generation {
            return;
        }
        let text = (self.buf.len_chars() <= PREVIEW_MAX_CHARS).then(|| self.buf.text.to_string());
        self.preview.set_blocks(text.as_deref(), self.buf.generation);
        unsafe {
            let _ = InvalidateRect(Some(self.prev), None, false);
        }
    }

    fn preview_timer(&mut self) {
        self.preview_pending = false;
        unsafe {
            let _ = KillTimer(Some(self.main), PREVIEW_TIMER);
        }
        self.update_preview();
    }

    fn paint_preview(&mut self) {
        let (w, h) = client_size(self.prev);
        self.preview.resize(w.max(1) as u32, h.max(1) as u32);
        self.preview.paint(&self.theme);
        set_scroll(
            self.prev,
            SB_VERT,
            self.preview.content_height(),
            self.preview.view_height(),
            self.preview.scroll_y,
        );
    }

    fn preview_scroll(&mut self, code: u16) {
        let step = self.settings.font_size * self.scale * 1.5;
        let Some(target) = scroll_target(self.prev, SB_VERT, code, self.preview.scroll_y, step) else { return };
        self.preview.scroll_to(target);
        unsafe {
            let _ = InvalidateRect(Some(self.prev), None, false);
        }
    }

    // the wheel goes to the focused window, so route it to whichever pane is under the cursor
    fn wheel(&mut self, delta: i16) {
        let over_preview = unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            WindowFromPoint(pt) == self.prev
        };
        if over_preview && !key_down(VK_CONTROL.0) {
            self.preview.scroll_by(-(delta as f32 / 120.0) * self.settings.font_size * self.scale * 4.5);
            unsafe {
                let _ = InvalidateRect(Some(self.prev), None, false);
            }
        } else {
            self.on_wheel(delta);
        }
    }

    fn link_under_cursor(&mut self) -> Option<String> {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.prev, &mut pt);
        }
        self.preview.link_at(pt.x as f32, pt.y as f32)
    }

    fn canvas_resize(&mut self) {
        let (w, h) = client_size(self.edit);
        self.canvas.resize(w.max(1) as u32, h.max(1) as u32);
        unsafe {
            let _ = InvalidateRect(Some(self.edit), None, false);
        }
    }

    fn paint_status(&mut self) {
        let mut ps = PAINTSTRUCT::default();
        unsafe {
            let hdc = BeginPaint(self.main, &mut ps);
            let (w, h) = client_size(self.main);
            let sh = self.status_height();
            let bar = RECT { left: 0, top: h - sh, right: w, bottom: h };
            let t = self.theme;
            fill(hdc, &bar, t.status_bg);
            fill(hdc, &RECT { left: 0, top: bar.top, right: w, bottom: bar.top + 1 }, t.rule);
            if self.settings.view == ViewMode::Split {
                fill(hdc, &RECT { left: w / 2 - 1, top: 0, right: w / 2, bottom: h - sh }, t.rule);
            }
            SelectObject(hdc, self.status_font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, colorref(t.status_fg));

            let (line, col) = self.buf.line_col(self.ed.cursor);
            let pad = (12.0 * self.scale) as i32;
            let chars = self.buf.len_chars();
            let zoom = (self.settings.font_size / Settings::default().font_size * 100.0).round();
            let left = [format!("Ln {line}, Col {col}"), format!("{} characters", group(chars)), format!("{zoom:.0}%")];
            let right = [(self.line_ending.label(), ID_LINE_ENDING), (self.encoding.label(), ID_ENCODING)];

            self.status_parts.clear();
            let mut x = pad;
            for text in &left {
                let width = text_width(hdc, text);
                let r = RECT { left: x, top: bar.top, right: x + width, bottom: bar.bottom };
                draw_text(hdc, text, r);
                x += width + pad * 2;
            }
            let mut x = w - pad;
            if self.settings.update_reminder {
                let dot = "\u{25CF}";
                let width = text_width(hdc, dot);
                let r = RECT { left: x - width, top: bar.top, right: x, bottom: bar.bottom };
                let color = match (self.update_due, t.dark) {
                    (false, _) => t.muted,
                    (true, false) => Rgb(214, 160, 0),
                    (true, true) => Rgb(240, 200, 60),
                };
                SetTextColor(hdc, colorref(color));
                draw_text(hdc, dot, r);
                SetTextColor(hdc, colorref(t.status_fg));
                self.status_parts.push((r, ID_UPDATE_DOT));
                x -= width + pad * 2;
            }
            for (text, id) in &right {
                let width = text_width(hdc, text);
                let r = RECT { left: x - width, top: bar.top, right: x, bottom: bar.bottom };
                draw_text(hdc, text, r);
                self.status_parts.push((r, *id));
                x -= width + pad * 2;
            }
            if let Some(note) = self.update_note {
                let width = text_width(hdc, note);
                draw_text(hdc, note, RECT { left: x - width, top: bar.top, right: x, bottom: bar.bottom });
            }
            let _ = EndPaint(self.main, &ps);
        }
    }

    fn status_click(&mut self, x: i32, y: i32) -> Option<u16> {
        let (_, h) = client_size(self.main);
        if y < h - self.status_height() {
            return None;
        }
        self.status_parts.iter().find(|(r, _)| x >= r.left - 8 && x <= r.right + 8).map(|(_, id)| *id)
    }
}

fn colorref(c: Rgb) -> COLORREF {
    COLORREF(c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16)
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn make_status_font(scale: f32) -> HFONT {
    unsafe {
        CreateFontW(
            (-12.0 * scale) as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            w!("Segoe UI"),
        )
    }
}

unsafe fn text_width(hdc: windows::Win32::Graphics::Gdi::HDC, text: &str) -> i32 {
    let mut w16: Vec<u16> = text.encode_utf16().collect();
    let mut r = RECT::default();
    DrawTextW(hdc, &mut w16, &mut r, DT_SINGLELINE | DT_NOPREFIX | windows::Win32::Graphics::Gdi::DT_CALCRECT);
    r.right - r.left
}

unsafe fn draw_text(hdc: windows::Win32::Graphics::Gdi::HDC, text: &str, mut r: RECT) {
    let mut w16: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut w16, &mut r, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

// shows an error from an app method, after the borrow is released
fn report<T>(r: Option<Result<T, String>>) -> Option<T> {
    match r {
        Some(Ok(v)) => Some(v),
        Some(Err(e)) => {
            message(APP, &e, rfd::MessageLevel::Error);
            None
        }
        None => None,
    }
}

fn parent() -> Option<Parent> {
    with_app(|a| Parent(a.main))
}

fn confirm_discard() -> bool {
    let Some((dirty, name)) = with_app(|a| (a.dirty, a.display_name())) else { return false };
    if !dirty {
        return true;
    }
    let mut d = rfd::MessageDialog::new()
        .set_title(APP)
        .set_description(format!("Do you want to save changes to {name}?"))
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom("Save".into(), "Don't save".into(), "Cancel".into()));
    if let Some(p) = parent() {
        d = d.set_parent(&p);
    }
    match d.show() {
        rfd::MessageDialogResult::Yes => save(),
        rfd::MessageDialogResult::Custom(s) if s == "Save" => save(),
        rfd::MessageDialogResult::No => true,
        rfd::MessageDialogResult::Custom(s) if s == "Don't save" => true,
        _ => false,
    }
}

fn save() -> bool {
    match with_app(|a| a.path.clone()).flatten() {
        Some(p) => report(with_app(|a| a.write_to(p))).is_some(),
        None => save_as(),
    }
}

fn save_as() -> bool {
    let current = with_app(|a| a.path.clone()).flatten();
    let mut dlg = rfd::FileDialog::new()
        .add_filter("Text document (*.txt)", &["txt"])
        .add_filter("Markdown (*.md)", &["md", "markdown"])
        .add_filter("All files", &["*"]);
    dlg = match &current {
        Some(p) => dlg
            .set_directory(p.parent().unwrap_or(Path::new(".")))
            .set_file_name(p.file_name().unwrap_or_default().to_string_lossy()),
        None => dlg.set_file_name("Untitled.txt"),
    };
    if let Some(p) = parent() {
        dlg = dlg.set_parent(&p);
    }
    let Some(mut path) = dlg.save_file() else { return false };
    if path.extension().is_none() {
        path.set_extension("txt");
    }
    report(with_app(|a| a.write_to(path))).is_some()
}

fn open() {
    if !confirm_discard() {
        return;
    }
    let dir = with_app(|a| a.path.as_ref().and_then(|p| p.parent().map(|d| d.to_owned()))).flatten();
    let mut dlg = rfd::FileDialog::new()
        .add_filter("Text and Markdown", &["txt", "md", "markdown"])
        .add_filter("All files", &["*"]);
    if let Some(d) = dir {
        dlg = dlg.set_directory(d);
    }
    if let Some(p) = parent() {
        dlg = dlg.set_parent(&p);
    }
    if let Some(path) = dlg.pick_file() {
        report(with_app(|a| a.load(&path)));
    }
}

fn open_path(path: &Path) {
    if confirm_discard() {
        report(with_app(|a| a.load(path)));
    }
}

fn uninstall() {
    let mut d = rfd::MessageDialog::new()
        .set_title(APP)
        .set_description(format!("Remove {APP}'s file associations, shortcut and installed copy?"))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(p) = parent() {
        d = d.set_parent(&p);
    }
    if d.show() == rfd::MessageDialogResult::Yes {
        match assoc::uninstall() {
            Ok(()) => message(APP, "Uninstalled. This window stays open until you close it.", rfd::MessageLevel::Info),
            Err(e) => message(APP, &e, rfd::MessageLevel::Error),
        }
    }
}

fn install() {
    match assoc::install() {
        Ok(exe) => message(
            APP,
            &format!("Installed to {}.\n\nTo make {APP} the default, use Open with > Always on a .md or .txt file (or Settings > Default apps).", exe.display()),
            rfd::MessageLevel::Info,
        ),
        Err(e) => message(APP, &e, rfd::MessageLevel::Error),
    }
}

fn ask(text: &str) -> bool {
    let mut d = rfd::MessageDialog::new().set_title(APP).set_description(text).set_buttons(rfd::MessageButtons::YesNo);
    if let Some(p) = parent() {
        d = d.set_parent(&p);
    }
    d.show() == rfd::MessageDialogResult::Yes
}

fn set_update_note(note: Option<&'static str>) {
    with_app(|a| {
        a.update_note = note;
        unsafe {
            let _ = InvalidateRect(Some(a.main), None, false);
        }
    });
}

// hands a result from the update thread to the window; dropped if the window is gone
fn post_to_main<T: 'static>(main: isize, msg: u32, value: T) {
    let ptr = Box::into_raw(Box::new(value));
    unsafe {
        if PostMessageW(Some(HWND(main as *mut _)), msg, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
}

// only ever started by the user, from the Help menu or the status bar dot
fn check_for_updates() {
    let Some(main) = with_app(|a| a.main.0 as isize) else { return };
    if update::spawn(update::latest, move |r| post_to_main(main, WM_UPDATE_CHECKED, r)) {
        set_update_note(Some("Checking for updates..."));
    }
}

fn on_update_checked(r: Result<update::Latest, String>) {
    with_app(|a| a.update_due &= r.is_err());
    set_update_note(None);
    let current = env!("CARGO_PKG_VERSION");
    match r {
        Err(e) => message(APP, &e, rfd::MessageLevel::Warning),
        Ok(update::Latest::Current) => {
            message(APP, &format!("{APP} {current} is the latest version."), rfd::MessageLevel::Info)
        }
        Ok(update::Latest::Newer(v)) => {
            let text = format!(
                "{APP} {v} is available (you have {current}).\n\nDownload it from github.com/{} and install it now? \
                 This window stays open; the new version starts the next time you open {APP}.",
                update::REPO
            );
            let Some(main) = with_app(|a| a.main.0 as isize) else { return };
            if ask(&text)
                && update::spawn(
                    move || update::install(&v).map(|()| v),
                    move |r| post_to_main(main, WM_UPDATE_INSTALLED, r),
                )
            {
                set_update_note(Some("Downloading update..."));
            }
        }
    }
}

fn on_update_installed(r: Result<String, String>) {
    set_update_note(None);
    match r {
        Ok(v) => {
            message(APP, &format!("{APP} {v} is installed. Close and reopen {APP} to use it."), rfd::MessageLevel::Info)
        }
        Err(e) => message(APP, &e, rfd::MessageLevel::Error),
    }
}

fn open_find(replace: bool) {
    let Some((main, prefill, query, case)) =
        with_app(|a| (a.main, a.selected_single_line(), a.find_query.clone(), a.find_case))
    else {
        return;
    };
    dialogs::show_find(main, replace, if prefill.is_empty() { &query } else { &prefill }, case);
}

fn not_found(query: &str) {
    message(APP, &format!("Cannot find \"{query}\""), rfd::MessageLevel::Info);
}

fn find_again(forward: bool) {
    let query = with_app(|a| a.find_query.clone()).unwrap_or_default();
    if query.is_empty() {
        open_find(false);
    } else if with_app(|a| a.find_next(forward)) == Some(false) {
        not_found(&query);
    }
}

fn goto_line() {
    let Some(main) = with_app(|a| a.main) else { return };
    if let Some(n) = dialogs::goto_line(main) {
        with_app(|a| a.go_to_line(n));
    }
}

fn on_find_message(lp: LPARAM) {
    let Some(req) = dialogs::on_find_message(lp) else { return };
    let found = with_app(|a| {
        a.find_query = req.query.clone();
        a.find_case = req.case_sensitive;
        match req.action {
            FindAction::Closed => true,
            FindAction::Next { forward } => a.find_next(forward),
            FindAction::Replace => a.replace_current(&req.replacement),
            FindAction::ReplaceAll => a.replace_all(&req.replacement) > 0,
        }
    });
    if found == Some(false) {
        not_found(&req.query);
    }
}

pub fn command(id: u16) {
    match id {
        ID_NEW => {
            if confirm_discard() {
                with_app(|a| a.new_doc());
            }
        }
        ID_NEW_WINDOW => {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
        }
        ID_OPEN => open(),
        ID_SAVE => {
            save();
        }
        ID_SAVE_AS => {
            save_as();
        }
        ID_EXIT => {
            if let Some(h) = with_app(|a| a.main) {
                unsafe {
                    let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        ID_UNDO => drop(with_app(|a| a.undo(false))),
        ID_REDO => drop(with_app(|a| a.undo(true))),
        ID_CUT => drop(with_app(|a| a.copy(true))),
        ID_COPY => drop(with_app(|a| a.copy(false))),
        ID_PASTE => drop(with_app(|a| a.paste())),
        ID_SELECT_ALL => drop(with_app(|a| a.select_all())),
        ID_TIME_DATE => drop(with_app(|a| a.insert(&settings::local_time_stamp()))),
        ID_WRAP => {
            with_app(|a| {
                a.settings.wrap = !a.settings.wrap;
                unsafe {
                    let _ = ShowScrollBar(a.edit, SB_HORZ, !a.settings.wrap);
                }
                a.layout_children();
                a.refresh();
            });
        }
        ID_FIND | ID_REPLACE => open_find(id == ID_REPLACE),
        ID_FIND_NEXT | ID_FIND_PREV => find_again(id == ID_FIND_NEXT),
        ID_GOTO => goto_line(),
        ID_THEME_SYSTEM | ID_THEME_LIGHT | ID_THEME_DARK => drop(with_app(|a| {
            a.settings.theme = match id {
                ID_THEME_LIGHT => ThemeMode::Light,
                ID_THEME_DARK => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            a.apply_theme();
            a.settings.save();
        })),
        ID_ZOOM_IN => drop(with_app(|a| a.zoom(1.1))),
        ID_ZOOM_OUT => drop(with_app(|a| a.zoom(1.0 / 1.1))),
        ID_ZOOM_RESET => drop(with_app(|a| a.reset_zoom())),
        ID_INSTALL => install(),
        ID_UNINSTALL => uninstall(),
        ID_CHECK_UPDATES => check_for_updates(),
        ID_UPDATE_REMINDER => drop(with_app(|a| {
            a.settings.update_reminder = !a.settings.update_reminder;
            a.update_due = a.settings.update_reminder && update::reminder_due();
            a.settings.save();
            unsafe {
                let _ = InvalidateRect(Some(a.main), None, false);
            }
        })),
        ID_ABOUT => message(
            APP,
            &format!("{APP} {}\nA small editor for .txt and .md files.", env!("CARGO_PKG_VERSION")),
            rfd::MessageLevel::Info,
        ),
        id if (ID_ENCODING..ID_ENCODING + 5).contains(&id) => {
            with_app(|a| {
                a.encoding = Encoding::ALL[(id - ID_ENCODING) as usize];
                a.dirty = true;
                a.refresh();
            });
        }
        id if (ID_LINE_ENDING..ID_LINE_ENDING + 2).contains(&id) => {
            with_app(|a| {
                a.line_ending = if id == ID_LINE_ENDING { LineEnding::Crlf } else { LineEnding::Lf };
                a.dirty = true;
                a.refresh();
            });
        }
        _ => {}
    }
}

unsafe fn add(menu: HMENU, id: u16, text: &str) {
    let _ = AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(wide(text).as_ptr()));
}

unsafe fn sep(menu: HMENU) {
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
}

unsafe fn sub(bar: HMENU, menu: HMENU, text: &str) {
    let _ = AppendMenuW(bar, MF_POPUP, menu.0 as usize, PCWSTR(wide(text).as_ptr()));
}

fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu().unwrap();
        let file = CreatePopupMenu().unwrap();
        add(file, ID_NEW, "New\tCtrl+N");
        add(file, ID_NEW_WINDOW, "New window\tCtrl+Shift+N");
        add(file, ID_OPEN, "Open...\tCtrl+O");
        add(file, ID_SAVE, "Save\tCtrl+S");
        add(file, ID_SAVE_AS, "Save as...\tCtrl+Shift+S");
        sep(file);
        add(file, ID_EXIT, "Exit");
        sub(bar, file, "&File");

        let edit = CreatePopupMenu().unwrap();
        add(edit, ID_UNDO, "Undo\tCtrl+Z");
        add(edit, ID_REDO, "Redo\tCtrl+Y");
        sep(edit);
        add(edit, ID_CUT, "Cut\tCtrl+X");
        add(edit, ID_COPY, "Copy\tCtrl+C");
        add(edit, ID_PASTE, "Paste\tCtrl+V");
        sep(edit);
        add(edit, ID_FIND, "Find...\tCtrl+F");
        add(edit, ID_FIND_NEXT, "Find next\tF3");
        add(edit, ID_FIND_PREV, "Find previous\tShift+F3");
        add(edit, ID_REPLACE, "Replace...\tCtrl+H");
        add(edit, ID_GOTO, "Go to line...\tCtrl+G");
        sep(edit);
        add(edit, ID_SELECT_ALL, "Select all\tCtrl+A");
        add(edit, ID_TIME_DATE, "Time/Date\tF5");
        sub(bar, edit, "&Edit");

        let format = CreatePopupMenu().unwrap();
        add(format, ID_WRAP, "Word wrap");
        sub(bar, format, "F&ormat");

        let view = CreatePopupMenu().unwrap();
        add(view, ID_VIEW_EDITOR, "Editor only");
        add(view, ID_VIEW_SPLIT, "Editor and formatted preview\tCtrl+E");
        add(view, ID_VIEW_PREVIEW, "Formatted preview only\tCtrl+Shift+E");
        sep(view);
        let themes = CreatePopupMenu().unwrap();
        add(themes, ID_THEME_SYSTEM, "Use system setting");
        add(themes, ID_THEME_LIGHT, "Light");
        add(themes, ID_THEME_DARK, "Dark");
        sub(view, themes, "&Theme");
        sep(view);
        add(view, ID_ZOOM_IN, "Zoom in\tCtrl++");
        add(view, ID_ZOOM_OUT, "Zoom out\tCtrl+-");
        add(view, ID_ZOOM_RESET, "Restore default zoom\tCtrl+0");
        sub(bar, view, "&View");

        let help = CreatePopupMenu().unwrap();
        add(help, ID_INSTALL, "Install for this user (file associations)");
        add(help, ID_UNINSTALL, "Uninstall");
        sep(help);
        add(help, ID_CHECK_UPDATES, "Check for updates (connects to github.com)");
        add(help, ID_UPDATE_REMINDER, "Remind me to check for updates");
        sep(help);
        add(help, ID_ABOUT, &format!("About {APP}"));
        sub(bar, help, "&Help");
        bar
    }
}

fn accelerators() -> HACCEL {
    let ctrl = FCONTROL | FVIRTKEY;
    let cs = FCONTROL | FSHIFT | FVIRTKEY;
    let mk = |virt: ACCEL_VIRT_FLAGS, key: u16, cmd: u16| ACCEL { fVirt: virt, key, cmd };
    let table = [
        mk(ctrl, b'N' as u16, ID_NEW),
        mk(cs, b'N' as u16, ID_NEW_WINDOW),
        mk(ctrl, b'O' as u16, ID_OPEN),
        mk(ctrl, b'S' as u16, ID_SAVE),
        mk(cs, b'S' as u16, ID_SAVE_AS),
        mk(ctrl, b'F' as u16, ID_FIND),
        mk(ctrl, b'H' as u16, ID_REPLACE),
        mk(ctrl, b'G' as u16, ID_GOTO),
        mk(ctrl, b'E' as u16, ID_TOGGLE_PREVIEW),
        mk(cs, b'E' as u16, ID_CYCLE_PREVIEW),
        mk(FVIRTKEY, VK_F3.0, ID_FIND_NEXT),
        mk(FSHIFT | FVIRTKEY, VK_F3.0, ID_FIND_PREV),
        mk(FVIRTKEY, VK_F5.0, ID_TIME_DATE),
        mk(ctrl, VK_OEM_PLUS.0, ID_ZOOM_IN),
        mk(ctrl, VK_ADD.0, ID_ZOOM_IN),
        mk(ctrl, VK_OEM_MINUS.0, ID_ZOOM_OUT),
        mk(ctrl, VK_SUBTRACT.0, ID_ZOOM_OUT),
        mk(ctrl, b'0' as u16, ID_ZOOM_RESET),
    ];
    unsafe { CreateAcceleratorTableW(&table).unwrap() }
}

fn update_menu_checks(main: HWND) {
    let Some((wrap, view, theme, reminder)) =
        with_app(|a| (a.settings.wrap, a.settings.view, a.settings.theme, a.settings.update_reminder))
    else {
        return;
    };
    unsafe {
        let menu = GetMenu(main);
        let check = |id: u16, on: bool| {
            CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
        };
        check(ID_WRAP, wrap);
        check(ID_VIEW_EDITOR, view == ViewMode::Editor);
        check(ID_VIEW_SPLIT, view == ViewMode::Split);
        check(ID_VIEW_PREVIEW, view == ViewMode::Preview);
        check(ID_THEME_SYSTEM, theme == ThemeMode::System);
        check(ID_THEME_LIGHT, theme == ThemeMode::Light);
        check(ID_THEME_DARK, theme == ThemeMode::Dark);
        check(ID_UPDATE_REMINDER, reminder);
        let installed = assoc::is_installed();
        let _ =
            EnableMenuItem(menu, ID_UNINSTALL as u32, MF_BYCOMMAND | if installed { MF_ENABLED } else { MF_GRAYED });
    }
}

unsafe fn popup(hwnd: HWND, items: &[(u16, &str, bool)], x: i32, y: i32, align: TRACK_POPUP_MENU_FLAGS) {
    let menu = CreatePopupMenu().unwrap();
    for (id, text, checked) in items {
        let flags = MF_STRING | if *checked { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, flags, *id as usize, PCWSTR(wide(text).as_ptr()));
    }
    let _ = TrackPopupMenu(menu, align, x, y, Some(0), hwnd, None);
    let _ = DestroyMenu(menu);
}

fn status_popup(main: HWND, id: u16, x: i32, y: i32) {
    let Some((enc, le)) = with_app(|a| (a.encoding, a.line_ending)) else { return };
    let items: Vec<(u16, &str, bool)> = if id == ID_UPDATE_DOT {
        vec![
            (ID_CHECK_UPDATES, "Check for updates (connects to github.com)", false),
            (ID_UPDATE_REMINDER, "Remind me to check for updates", true),
        ]
    } else if id == ID_ENCODING {
        Encoding::ALL.iter().enumerate().map(|(i, e)| (ID_ENCODING + i as u16, e.label(), *e == enc)).collect()
    } else {
        [LineEnding::Crlf, LineEnding::Lf]
            .iter()
            .enumerate()
            .map(|(i, l)| (ID_LINE_ENDING + i as u16, l.label(), *l == le))
            .collect()
    };
    let mut pt = POINT { x, y };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(main, &mut pt);
        popup(main, &items, pt.x, pt.y, TPM_LEFTALIGN | TPM_BOTTOMALIGN);
    }
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let find_msg = dialogs::find_message();
    if find_msg != 0 && msg == find_msg {
        on_find_message(lp);
        return LRESULT(0);
    }
    unsafe {
        match msg {
            WM_SIZE => {
                if wp.0 == 1 {
                    trim_working_set();
                }
                with_app(|a| a.layout_children());
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == PREVIEW_TIMER {
                    with_app(|a| a.preview_timer());
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = hiword(wp.0) as i16;
                with_app(|a| a.wheel(delta));
                LRESULT(0)
            }
            WM_UAHDRAWMENU => match dark_menu() {
                Some((t, _)) => {
                    draw_menu_bar(hwnd, lp, &t);
                    LRESULT(1)
                }
                None => DefWindowProcW(hwnd, msg, wp, lp),
            },
            WM_UAHDRAWMENUITEM => match dark_menu() {
                Some((t, font)) => {
                    draw_menu_item(lp, &t, font);
                    LRESULT(1)
                }
                None => DefWindowProcW(hwnd, msg, wp, lp),
            },
            WM_NCPAINT | WM_NCACTIVATE => {
                let r = DefWindowProcW(hwnd, msg, wp, lp);
                if let Some((t, _)) = dark_menu() {
                    paint_menu_line(hwnd, t.status_bg);
                }
                r
            }
            WM_SETTINGCHANGE => {
                let is_theme =
                    lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet");
                if is_theme {
                    with_app(|a| {
                        if a.settings.theme == ThemeMode::System {
                            a.apply_theme();
                        }
                    });
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                if with_app(|a| a.paint_status()).is_none() {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            WM_SETFOCUS => {
                if let Some(e) = with_app(|a| a.edit) {
                    let _ = SetFocus(Some(e));
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = loword(wp.0);
                if id == ID_TOGGLE_PREVIEW || id == ID_CYCLE_PREVIEW || (ID_VIEW_EDITOR..=ID_VIEW_PREVIEW).contains(&id)
                {
                    with_app(|a| match id {
                        ID_VIEW_EDITOR => a.set_view(ViewMode::Editor),
                        ID_VIEW_SPLIT => a.set_view(ViewMode::Split),
                        ID_VIEW_PREVIEW => a.set_view(ViewMode::Preview),
                        ID_CYCLE_PREVIEW => a.cycle_preview(),
                        _ => a.toggle_preview(),
                    });
                } else {
                    command(id);
                }
                LRESULT(0)
            }
            WM_INITMENUPOPUP => {
                update_menu_checks(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (loword(lp.0 as usize) as i16 as i32, hiword(lp.0 as usize) as i16 as i32);
                if let Some(Some(id)) = with_app(|a| a.status_click(x, y)) {
                    status_popup(hwnd, id, x, y);
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                let hdrop = HDROP(wp.0 as *mut _);
                let mut buf = [0u16; 1024];
                let n = DragQueryFileW(hdrop, 0, Some(&mut buf)) as usize;
                DragFinish(hdrop);
                if n > 0 {
                    open_path(Path::new(&sys::from_wide(&buf[..n])));
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                with_app(|a| {
                    a.scale = hiword(wp.0) as f32 / 96.0;
                    let _ = DeleteObject(a.status_font.into());
                    a.status_font = make_status_font(a.scale);
                    a.rebuild_font();
                    a.layout_children();
                });
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                mmi.ptMinTrackSize = POINT { x: 360, y: 240 };
                LRESULT(0)
            }
            WM_CLOSE => {
                if confirm_discard() {
                    with_app(|a| {
                        let mut r = RECT::default();
                        let _ = GetWindowRect(a.main, &mut r);
                        a.settings.window_size = [
                            ((r.right - r.left) as f32 / a.scale).max(360.0),
                            ((r.bottom - r.top) as f32 / a.scale).max(240.0),
                        ];
                        a.settings.save();
                    });
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_UPDATE_CHECKED => {
                on_update_checked(*Box::from_raw(lp.0 as *mut Result<update::Latest, String>));
                LRESULT(0)
            }
            WM_UPDATE_INSTALLED => {
                on_update_installed(*Box::from_raw(lp.0 as *mut Result<String, String>));
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let xy = || (loword(lp.0 as usize) as i16 as i32, hiword(lp.0 as usize) as i16 as i32);
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                with_app(|a| a.paint_edit());
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                with_app(|a| a.canvas_resize());
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let handled = with_app(|a| a.on_key(wp.0 as u16)).unwrap_or(false);
                if handled {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_CHAR => {
                with_app(|a| a.on_char(wp.0 as u16));
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = xy();
                with_app(|a| a.on_mouse_down(x, y, msg == WM_LBUTTONDBLCLK));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy();
                with_app(|a| a.on_mouse_move(x, y));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                with_app(|a| a.on_mouse_up());
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = hiword(wp.0) as i16;
                with_app(|a| a.wheel(delta));
                LRESULT(0)
            }
            WM_VSCROLL | WM_HSCROLL => {
                with_app(|a| a.on_scroll(msg == WM_VSCROLL, loword(wp.0)));
                LRESULT(0)
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                if msg == WM_SETFOCUS {
                    with_app(|a| a.restart_blink());
                } else {
                    let _ = KillTimer(Some(hwnd), BLINK_TIMER);
                    with_app(|a| a.blink_running = false);
                    trim_working_set();
                }
                LRESULT(0)
            }
            WM_TIMER => {
                with_app(|a| a.blink_tick());
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_SETCURSOR if loword(lp.0 as usize) as u32 == HTCLIENT => {
                SetCursor(Some(LoadCursorW(None, IDC_IBEAM).unwrap()));
                LRESULT(1)
            }
            WM_IME_STARTCOMPOSITION => {
                with_app(|a| a.place_ime());
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CONTEXTMENU => {
                let (mut x, mut y) = (loword(lp.0 as usize) as i16 as i32, hiword(lp.0 as usize) as i16 as i32);
                if x == -1 && y == -1 {
                    x = 20;
                    y = 20;
                }
                let items = [
                    (ID_UNDO, "Undo", false),
                    (ID_CUT, "Cut", false),
                    (ID_COPY, "Copy", false),
                    (ID_PASTE, "Paste", false),
                    (ID_SELECT_ALL, "Select all", false),
                ];
                popup(GetParent(hwnd).unwrap_or_default(), &items, x, y, TPM_LEFTALIGN | TPM_TOPALIGN);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// moves the process's resident pages to the standby list; they come back on demand, so an idle editor holds little RAM
fn trim_working_set() {
    unsafe {
        let _ = windows::Win32::System::Memory::SetProcessWorkingSetSizeEx(
            windows::Win32::Foundation::HANDLE(-1isize as *mut std::ffi::c_void),
            usize::MAX,
            usize::MAX,
            windows::Win32::System::Memory::SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        );
    }
}

// undocumented but long-stable uxtheme entry points: 0 default, 1 allow dark, 2 force dark, 3 force light
unsafe fn set_app_mode(mode: i32) {
    let Ok(m) = LoadLibraryW(w!("uxtheme.dll")) else { return };
    if let Some(f) = GetProcAddress(m, windows::core::PCSTR(135 as *const u8)) {
        let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_mode(mode);
    }
    if let Some(f) = GetProcAddress(m, windows::core::PCSTR(136 as *const u8)) {
        let flush: extern "system" fn() = std::mem::transmute(f);
        flush();
    }
}

fn app_mode(mode: ThemeMode) -> i32 {
    match mode {
        ThemeMode::System => 1,
        ThemeMode::Light => 3,
        ThemeMode::Dark => 2,
    }
}

fn apply_window_theme(main: HWND, edit: HWND, prev: HWND, theme: &Theme, mode: ThemeMode, live: bool) {
    unsafe {
        set_app_mode(app_mode(mode));
        let on: i32 = theme.dark as i32;
        let _ = DwmSetWindowAttribute(main, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as *const _, 4);
        let name = if theme.dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
        for h in [edit, prev] {
            let _ = SetWindowTheme(h, name, PCWSTR::null());
        }
        // only a live theme switch needs the frame recalculated; at startup the window is not shown yet
        if live {
            let _ = DrawMenuBar(main);
            let _ = SetWindowPos(main, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        }
    }
}

unsafe fn fill(hdc: windows::Win32::Graphics::Gdi::HDC, r: &RECT, c: Rgb) {
    let b = CreateSolidBrush(colorref(c));
    FillRect(hdc, r, b);
    let _ = DeleteObject(b.into());
}

// The native menu bar ignores dark mode, so in dark themes it is painted here. These messages are undocumented
// but are what the shell itself uses; if they ever stop arriving the bar simply stays light.
const WM_UAHDRAWMENU: u32 = 0x0091;
const WM_UAHDRAWMENUITEM: u32 = 0x0092;

#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: windows::Win32::Graphics::Gdi::HDC,
    flags: u32,
}

#[repr(C)]
struct UahMenuItem {
    position: i32,
    metrics: [u32; 8],
    popup_metrics: [u32; 5],
}

#[repr(C)]
struct UahDrawMenuItem {
    dis: windows::Win32::UI::Controls::DRAWITEMSTRUCT,
    menu: UahMenu,
    item: UahMenuItem,
}

fn dark_menu() -> Option<(Theme, HFONT)> {
    with_app(|a| a.theme.dark.then_some((a.theme, a.status_font))).flatten()
}

unsafe fn draw_menu_bar(hwnd: HWND, lp: LPARAM, t: &Theme) {
    let um = &*(lp.0 as *const UahMenu);
    let mut mbi = MENUBARINFO { cbSize: std::mem::size_of::<MENUBARINFO>() as u32, ..Default::default() };
    if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi).is_err() {
        return;
    }
    let mut wr = RECT::default();
    let _ = GetWindowRect(hwnd, &mut wr);
    let bar = RECT {
        left: mbi.rcBar.left - wr.left,
        top: mbi.rcBar.top - wr.top,
        right: mbi.rcBar.right - wr.left,
        bottom: mbi.rcBar.bottom - wr.top,
    };
    fill(um.hdc, &bar, t.status_bg);
}

unsafe fn draw_menu_item(lp: LPARAM, t: &Theme, font: HFONT) {
    let d = &*(lp.0 as *const UahDrawMenuItem);
    let mut text = [0u16; 64];
    let mut mii = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: windows::core::PWSTR(text.as_mut_ptr()),
        cch: 63,
        ..Default::default()
    };
    let _ = GetMenuItemInfoW(d.menu.hmenu, d.item.position as u32, true, &mut mii);
    const ODS_SELECTED: u32 = 0x0001;
    const ODS_HOTLIGHT: u32 = 0x0040;
    let hot = d.dis.itemState.0 & (ODS_SELECTED | ODS_HOTLIGHT) != 0;
    fill(d.menu.hdc, &d.dis.rcItem, if hot { t.hot } else { t.status_bg });
    SelectObject(d.menu.hdc, font.into());
    SetBkMode(d.menu.hdc, TRANSPARENT);
    SetTextColor(d.menu.hdc, colorref(t.status_fg));
    let mut rc = d.dis.rcItem;
    let len = (mii.cch as usize).min(63);
    DrawTextW(d.menu.hdc, &mut text[..len], &mut rc, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_HIDEPREFIX);
}

// the 1px light line Windows draws between the menu bar and the client area
unsafe fn paint_menu_line(hwnd: HWND, color: Rgb) {
    let mut wr = RECT::default();
    let _ = GetWindowRect(hwnd, &mut wr);
    let mut origin = POINT { x: 0, y: 0 };
    let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin);
    let (cw, _) = client_size(hwnd);
    let line = RECT {
        left: origin.x - wr.left,
        top: origin.y - wr.top - 1,
        right: origin.x - wr.left + cw,
        bottom: origin.y - wr.top,
    };
    let hdc = GetWindowDC(Some(hwnd));
    fill(hdc, &line, color);
    ReleaseDC(Some(hwnd), hdc);
}

extern "system" fn preview_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                with_app(|a| a.paint_preview());
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                with_app(|a| {
                    let (w, h) = client_size(hwnd);
                    a.preview.resize(w.max(1) as u32, h.max(1) as u32);
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = hiword(wp.0) as i16;
                with_app(|a| a.wheel(delta));
                LRESULT(0)
            }
            WM_VSCROLL => {
                with_app(|a| a.preview_scroll(loword(wp.0)));
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(Some(url)) = with_app(|a| a.link_under_cursor()) {
                    let _ = ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR(wide(&url).as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
                LRESULT(0)
            }
            WM_SETCURSOR if loword(lp.0 as usize) as u32 == HTCLIENT => {
                let link = with_app(|a| a.link_under_cursor()).flatten().is_some();
                SetCursor(Some(LoadCursorW(None, if link { IDC_HAND } else { IDC_ARROW }).unwrap()));
                LRESULT(1)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

pub fn run(pending: Option<(PathBuf, std::thread::JoinHandle<Result<doc::Loaded, String>>)>, settings: Settings) {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        set_app_mode(app_mode(settings.theme));
        let icon = LoadIconW(Some(hinst.into()), PCWSTR(1 as *const u16)).unwrap_or_default();
        let theme = Theme::for_mode(settings.theme);
        let bg = CreateSolidBrush(colorref(theme.bg));

        let main_class = w!("dvnoteMain");
        let edit_class = w!("dvnoteEdit");
        let wc_main = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst.into(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: bg,
            lpszClassName: main_class,
            ..Default::default()
        };
        RegisterClassExW(&wc_main);
        let wc_edit = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(edit_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_IBEAM).unwrap(),
            lpszClassName: edit_class,
            ..Default::default()
        };
        RegisterClassExW(&wc_edit);
        let prev_class = w!("dvnotePreview");
        let wc_prev = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(preview_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            lpszClassName: prev_class,
            ..Default::default()
        };
        RegisterClassExW(&wc_prev);

        let (w0, h0) = (settings.window_size[0] as i32, settings.window_size[1] as i32);
        let main = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            main_class,
            w!("dvnote"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            w0,
            h0,
            None,
            Some(build_menu()),
            Some(hinst.into()),
            None,
        )
        .unwrap();
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            edit_class,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_HSCROLL | WS_CLIPSIBLINGS,
            0,
            0,
            100,
            100,
            Some(main),
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap();
        let prev = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            prev_class,
            w!(""),
            WS_CHILD | WS_VSCROLL | WS_CLIPSIBLINGS,
            0,
            0,
            100,
            100,
            Some(main),
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap();
        apply_window_theme(main, edit, prev, &theme, settings.theme, false);

        let gfx = Gfx::new().expect("DirectWrite / Direct2D unavailable");
        let app = App::new(main, edit, prev, gfx, settings).expect("fonts");
        let wrap = app.settings.wrap;
        APP_CELL.with(|c| *c.borrow_mut() = Some(app));
        let _ = ShowScrollBar(edit, SB_HORZ, !wrap);
        with_app(|a| a.layout_children());
        if let Some((p, handle)) = pending {
            let loaded = handle.join().unwrap_or_else(|_| Err("The file could not be read.".to_owned()));
            report(with_app(|a| loaded.and_then(|l| a.open(&p, l))));
        }
        with_app(|a| a.refresh());
        // draw the first frame before the window appears, so it never shows blank
        with_app(|a| {
            a.paint_edit();
            if a.settings.view != ViewMode::Editor {
                a.paint_preview();
            }
        });
        let _ = ShowWindow(main, SW_SHOW);
        let _ = SetFocus(Some(edit));

        dialogs::register_messages();
        let accel = accelerators();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if let Some(d) = dialogs::dialog_hwnd() {
                if IsDialogMessageW(d, &msg).as_bool() {
                    continue;
                }
            }
            if TranslateAcceleratorW(main, accel, &msg) == 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = DestroyAcceleratorTable(accel);
        let _ = DeleteObject(bg.into());
    }
}
