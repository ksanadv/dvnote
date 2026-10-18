// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
//! dvnote's setup: the shared setup window around the program it carries, or with `--quiet` the same install with no
//! window. `--preview` walks through the window without installing anything.
#![windows_subsystem = "windows"]

#[allow(dead_code)]
#[path = "../../src/assoc.rs"]
mod assoc;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};

const PAYLOAD: &[u8] = include_bytes!(env!("DVNOTE_EXE"));
const VERSION: &str = env!("CARGO_PKG_VERSION");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn message(error: bool, text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("dvnote Setup").as_ptr(), MB_OK | icon) };
}

/// Windows' Default apps page, where dvnote can be made the default for .txt and .md
fn open_default_apps() {
    let _ = std::process::Command::new("cmd")
        .raw_arg(format!("/C start \"\" \"ms-settings:defaultapps?registeredAppUser={}\"", assoc::APP_NAME))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

struct Install;

impl dv_setup::Work for Install {
    /// the one box: open Default apps when done
    fn install(&self, ticked: &[bool], step: &dyn Fn(&str, f32)) -> Result<PathBuf, String> {
        let dir = assoc::install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        let target = dir.join(assoc::EXE_NAME);
        step("Copying dvnote.exe", 0.2);
        assoc::replace_exe(&target, |t| std::fs::write(t, PAYLOAD))?;
        step("Registering .txt and .md", 0.7);
        assoc::finish_install(&target)?;
        if ticked.first() == Some(&true) {
            open_default_apps();
        }
        step("Done", 1.0);
        Ok(target)
    }

    fn launch(&self, exe: &Path) {
        let _ = std::process::Command::new(exe).spawn();
    }
}

/// the window's pages with nothing written: each step only waits
struct Preview;

impl dv_setup::Work for Preview {
    fn install(&self, _: &[bool], step: &dyn Fn(&str, f32)) -> Result<PathBuf, String> {
        for (t, f) in [("Copying dvnote.exe", 0.2), ("Registering .txt and .md", 0.7)] {
            step(t, f);
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        Ok(assoc::install_dir().join(assoc::EXE_NAME))
    }

    fn launch(&self, _: &Path) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x.eq_ignore_ascii_case(a));
    let quiet = has("--quiet") || has("/S");
    if has("--uninstall") {
        match assoc::uninstall() {
            Ok(()) if !quiet => message(false, "dvnote has been removed."),
            Ok(()) => {}
            Err(e) => {
                if !quiet {
                    message(true, &e);
                }
                std::process::exit(1);
            }
        }
        return;
    }
    if quiet {
        let r = dv_setup::Work::install(&Install, &[false], &|_, _| {});
        std::process::exit(r.is_err() as i32);
    }
    let installed = assoc::installed_version();
    let mut done = "Find it in the Start menu, or open a .txt or .md file with it.".to_string();
    if installed.is_some() {
        done += "\n\nIf it is open, close and reopen it to use the new version.";
    }
    let product = dv_setup::Product {
        name: "dvnote",
        about: "A small editor for .txt and .md files",
        location: assoc::install_dir(),
        choices: vec![dv_setup::Choice {
            label: "Open Default apps afterwards, to make dvnote the default for .txt and .md".into(),
            on: installed.is_none(),
            needs: None,
        }],
        scope: dv_setup::Scope::App { version: VERSION, installed },
        done,
        launch: Some("Open dvnote now".into()),
        activate: std::env::var_os("DVNOTE_NOACTIVATE").is_none(),
    };
    if has("--preview") {
        dv_setup::run(product, Preview);
    } else {
        dv_setup::run(product, Install);
    }
}
