// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::sys::{from_wide, wide};
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::Dialogs::{
    FindTextW, ReplaceTextW, FINDMSGSTRINGW, FINDREPLACEW, FINDREPLACE_FLAGS, FR_DIALOGTERM, FR_DOWN, FR_FINDNEXT,
    FR_MATCHCASE, FR_REPLACE, FR_REPLACEALL,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const FIND_MAX: usize = 512;

pub enum FindAction {
    Next { forward: bool },
    Replace,
    ReplaceAll,
    Closed,
}

pub struct FindRequest {
    pub action: FindAction,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
}

// the struct and both buffers must outlive the modeless dialog, so they live here rather than on the stack
struct FindState {
    fr: Box<FINDREPLACEW>,
    find: Vec<u16>,
    replace: Vec<u16>,
    dialog: Option<HWND>,
}

thread_local! {
    static FIND: RefCell<Option<FindState>> = const { RefCell::new(None) };
    // kept outside the RefCell: the window procedure reads it while FindTextW is still running
    static FIND_MSG: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub fn find_message() -> u32 {
    FIND_MSG.with(|m| m.get())
}

pub fn dialog_hwnd() -> Option<HWND> {
    FIND.with(|f| f.borrow().as_ref().and_then(|s| s.dialog))
}

pub fn register_messages() {
    let msg = unsafe { RegisterWindowMessageW(FINDMSGSTRINGW) };
    FIND_MSG.with(|m| m.set(msg));
    FIND.with(|f| {
        *f.borrow_mut() = Some(FindState {
            fr: Box::new(FINDREPLACEW::default()),
            find: vec![0; FIND_MAX],
            replace: vec![0; FIND_MAX],
            dialog: None,
        })
    });
}

pub fn show_find(owner: HWND, replace: bool, prefill: &str, case_sensitive: bool) {
    // the borrow ends before the dialog is created: creating it sends messages to the owner synchronously
    let params = FIND.with(|f| {
        let mut guard = f.borrow_mut();
        let s = guard.as_mut()?;
        if let Some(d) = s.dialog {
            unsafe {
                let _ = SetForegroundWindow(d);
            }
            return None;
        }
        if !prefill.is_empty() {
            let w = wide(prefill);
            let n = w.len().min(FIND_MAX);
            s.find[..n].copy_from_slice(&w[..n]);
            s.find[n - 1] = 0;
        }
        let mut flags = FR_DOWN;
        if case_sensitive {
            flags |= FR_MATCHCASE;
        }
        *s.fr = FINDREPLACEW {
            lStructSize: std::mem::size_of::<FINDREPLACEW>() as u32,
            hwndOwner: owner,
            Flags: flags,
            lpstrFindWhat: windows::core::PWSTR(s.find.as_mut_ptr()),
            lpstrReplaceWith: windows::core::PWSTR(s.replace.as_mut_ptr()),
            wFindWhatLen: FIND_MAX as u16,
            wReplaceWithLen: FIND_MAX as u16,
            ..Default::default()
        };
        Some(&mut *s.fr as *mut FINDREPLACEW)
    });
    let Some(p) = params else { return };
    let hwnd = unsafe {
        if replace {
            ReplaceTextW(p)
        } else {
            FindTextW(p)
        }
    };
    FIND.with(|f| {
        if let Some(s) = f.borrow_mut().as_mut() {
            s.dialog = (!hwnd.is_invalid()).then_some(hwnd);
        }
    });
}

pub fn on_find_message(lparam: LPARAM) -> Option<FindRequest> {
    let fr = unsafe { &*(lparam.0 as *const FINDREPLACEW) };
    let flags: FINDREPLACE_FLAGS = fr.Flags;
    let query = unsafe { from_wide(std::slice::from_raw_parts(fr.lpstrFindWhat.0, FIND_MAX)) };
    let replacement = unsafe { from_wide(std::slice::from_raw_parts(fr.lpstrReplaceWith.0, FIND_MAX)) };
    let case_sensitive = flags.contains(FR_MATCHCASE);
    let action = if flags.contains(FR_DIALOGTERM) {
        FIND.with(|f| {
            if let Some(s) = f.borrow_mut().as_mut() {
                s.dialog = None;
            }
        });
        FindAction::Closed
    } else if flags.contains(FR_REPLACEALL) {
        FindAction::ReplaceAll
    } else if flags.contains(FR_REPLACE) {
        FindAction::Replace
    } else if flags.contains(FR_FINDNEXT) {
        FindAction::Next { forward: flags.contains(FR_DOWN) }
    } else {
        return None;
    };
    Some(FindRequest { action, query, replacement, case_sensitive })
}

const IDC_EDIT: i32 = 101;

fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_str(v: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16().chain(Some(0)) {
        push_u16(v, u);
    }
}

fn align4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn item(v: &mut Vec<u8>, style: u32, x: i16, y: i16, cx: i16, cy: i16, id: u16, class: u16, text: &str) {
    align4(v);
    push_u32(v, style | WS_CHILD.0 | WS_VISIBLE.0);
    push_u32(v, 0);
    for n in [x, y, cx, cy] {
        push_u16(v, n as u16);
    }
    push_u16(v, id);
    push_u16(v, 0xFFFF);
    push_u16(v, class);
    push_str(v, text);
    push_u16(v, 0);
}

fn goto_template() -> Vec<u8> {
    let mut v = Vec::new();
    let style = DS_MODALFRAME as u32 | DS_CENTER as u32 | DS_SETFONT as u32 | WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0;
    push_u32(&mut v, style);
    push_u32(&mut v, 0);
    push_u16(&mut v, 4);
    for n in [0i16, 0, 140, 58] {
        push_u16(&mut v, n as u16);
    }
    push_u16(&mut v, 0);
    push_u16(&mut v, 0);
    push_str(&mut v, "Go to line");
    push_u16(&mut v, 9);
    push_str(&mut v, "Segoe UI");
    const STATIC: u16 = 0x0082;
    const EDIT: u16 = 0x0081;
    const BUTTON: u16 = 0x0080;
    item(&mut v, 0, 10, 10, 120, 10, 0xFFFF, STATIC, "Line number:");
    item(
        &mut v,
        ES_NUMBER as u32 | WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
        10,
        22,
        120,
        12,
        IDC_EDIT as u16,
        EDIT,
        "",
    );
    item(&mut v, BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0, 30, 38, 50, 14, IDOK.0 as u16, BUTTON, "OK");
    item(&mut v, BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 84, 38, 46, 14, IDCANCEL.0 as u16, BUTTON, "Cancel");
    v
}

thread_local! {
    static GOTO_RESULT: RefCell<Option<usize>> = const { RefCell::new(None) };
}

unsafe extern "system" fn goto_proc(hdlg: HWND, msg: u32, wp: WPARAM, _lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let edit = GetDlgItem(Some(hdlg), IDC_EDIT).unwrap_or_default();
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
            1
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xFFFF) as i32;
            if id == IDOK.0 {
                let mut ok = Default::default();
                let n = GetDlgItemInt(hdlg, IDC_EDIT, Some(&mut ok), false);
                GOTO_RESULT.with(|r| *r.borrow_mut() = ok.as_bool().then_some(n as usize));
                let _ = EndDialog(hdlg, 1);
                1
            } else if id == IDCANCEL.0 {
                let _ = EndDialog(hdlg, 0);
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

pub fn goto_line(owner: HWND) -> Option<usize> {
    GOTO_RESULT.with(|r| *r.borrow_mut() = None);
    let template = goto_template();
    let mut aligned = vec![0u32; template.len().div_ceil(4)];
    unsafe {
        std::ptr::copy_nonoverlapping(template.as_ptr(), aligned.as_mut_ptr() as *mut u8, template.len());
        DialogBoxIndirectParamW(None, aligned.as_ptr() as *const DLGTEMPLATE, Some(owner), Some(goto_proc), LPARAM(0));
    }
    GOTO_RESULT.with(|r| r.borrow_mut().take())
}
