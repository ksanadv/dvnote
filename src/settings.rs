// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Editor,
    Split,
    Preview,
}

#[derive(Clone)]
pub struct Settings {
    pub wrap: bool,
    pub font_size: f32,
    // the view of the open document
    pub view: ViewMode,
    // the view Markdown files open in; the last one chosen while a Markdown file was open
    pub md_view: ViewMode,
    pub window_size: [f32; 2],
    pub theme: ThemeMode,
    // the status bar dot that turns yellow when updates have not been checked for a long time
    pub update_reminder: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wrap: true,
            font_size: 15.0,
            view: ViewMode::Editor,
            md_view: ViewMode::Editor,
            window_size: [900.0, 640.0],
            theme: ThemeMode::System,
            update_reminder: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(crate::assoc::APP_NAME).join("settings.txt"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "wrap" => s.wrap = v.trim() != "0",
                "update_reminder" => s.update_reminder = v.trim() != "0",
                "font_size" => s.font_size = v.trim().parse().map_or(s.font_size, |f: f32| f.clamp(8.0, 72.0)),
                "markdown_view" => {
                    s.md_view = match v.trim() {
                        "split" => ViewMode::Split,
                        "preview" => ViewMode::Preview,
                        _ => ViewMode::Editor,
                    }
                }
                "theme" => {
                    s.theme = match v.trim() {
                        "light" => ThemeMode::Light,
                        "dark" => ThemeMode::Dark,
                        _ => ThemeMode::System,
                    }
                }
                "width" => {
                    s.window_size[0] = v.trim().parse().map_or(s.window_size[0], |f: f32| f.clamp(320.0, 8000.0))
                }
                "height" => {
                    s.window_size[1] = v.trim().parse().map_or(s.window_size[1], |f: f32| f.clamp(200.0, 8000.0))
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        let view = match self.md_view {
            ViewMode::Editor => "editor",
            ViewMode::Split => "split",
            ViewMode::Preview => "preview",
        };
        let theme = match self.theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        let text = format!(
            "wrap={}\nfont_size={}\nmarkdown_view={view}\ntheme={theme}\nwidth={}\nheight={}\nupdate_reminder={}\n",
            self.wrap as u8,
            self.font_size,
            self.window_size[0].round(),
            self.window_size[1].round(),
            self.update_reminder as u8
        );
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, text);
    }
}

pub fn local_time_stamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    let hour12 = if t.wHour % 12 == 0 { 12 } else { t.wHour % 12 };
    let ampm = if t.wHour < 12 { "AM" } else { "PM" };
    format!("{hour12}:{:02} {ampm} {}/{}/{}", t.wMinute, t.wMonth, t.wDay, t.wYear)
}
