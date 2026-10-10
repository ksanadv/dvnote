// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
#![windows_subsystem = "windows"]

mod assoc;
#[cfg(test)]
mod bench;
mod buffer;
mod dialogs;
mod doc;
mod draw;
mod editor;
mod find;
mod md;
mod preview;
mod rowindex;
mod settings;
mod sys;
mod text;
mod trace;
mod update;
mod utf16;
mod win;

use settings::Settings;
use std::path::PathBuf;

const APP: &str = assoc::APP_NAME;

fn cli(result: Result<String, String>) -> ! {
    match result {
        Ok(text) => {
            win::message(APP, &text, rfd::MessageLevel::Info);
            std::process::exit(0)
        }
        Err(e) => {
            win::message(APP, &e, rfd::MessageLevel::Error);
            std::process::exit(1)
        }
    }
}

fn main() {
    trace::mark("main");
    std::thread::spawn(draw::warm_up);
    std::panic::set_hook(Box::new(|info| {
        sys::show_fatal(&format!("{APP} hit an internal error and must close.\n\n{info}"));
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--install") => cli(assoc::install().map(|exe| {
            format!(
                "{APP} is installed ({}).\n\nTo make it the default: right-click a .md or .txt file > Open with > Choose another app > {APP} > Always. Or use Settings > Apps > Default apps.",
                exe.display()
            )
        })),
        Some("--uninstall") => cli(assoc::uninstall().map(|()| format!("{APP} has been uninstalled."))),
        _ => {}
    }
    let path = args.into_iter().find(|a| !a.starts_with("--")).map(PathBuf::from);
    // start reading the file right away; the windows are created while it is decoded
    let pending = path.map(|p| {
        let q = p.clone();
        (p, std::thread::spawn(move || win::read_file(&q)))
    });
    win::run(pending, Settings::load());
}
