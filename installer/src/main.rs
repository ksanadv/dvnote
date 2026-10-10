// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
#![windows_subsystem = "windows"]

#[allow(dead_code)]
#[path = "../../src/assoc.rs"]
mod assoc;

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::os::windows::process::CommandExt;

const PAYLOAD: &[u8] = include_bytes!(env!("DVNOTE_EXE"));
const VERSION: &str = env!("CARGO_PKG_VERSION");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn dialog(quiet: bool, level: MessageLevel, text: &str) {
    if !quiet {
        MessageDialog::new().set_title("dvnote Setup").set_level(level).set_description(text).show();
    }
}

fn ask(text: &str) -> bool {
    let answer = MessageDialog::new()
        .set_title("dvnote Setup")
        .set_description(text)
        .set_buttons(MessageButtons::YesNo)
        .show();
    answer == MessageDialogResult::Yes
}

fn install() -> Result<std::path::PathBuf, String> {
    let dir = assoc::install_dir();
    let target = dir.join(assoc::EXE_NAME);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    assoc::replace_exe(&target, |t| std::fs::write(t, PAYLOAD))?;
    assoc::finish_install(&target)?;
    Ok(target)
}

fn question(installed: Option<&str>) -> String {
    let location = assoc::install_dir();
    match installed {
        None => format!(
            "Install dvnote {VERSION} for your user account?\n\nLocation: {}\nOpens: .txt  .md  .markdown\n\nNo administrator rights needed.",
            location.display()
        ),
        Some(v) if v == VERSION => format!("dvnote {v} is already installed.\n\nReinstall it? This repairs the program files and file associations."),
        Some(v) if assoc::is_newer(v, VERSION) => {
            format!("A newer dvnote ({v}) is installed.\n\nReplace it with the older version {VERSION}?")
        }
        Some(v) => format!("dvnote {v} is installed.\n\nUpdate it to {VERSION}? Your settings are kept."),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a.eq_ignore_ascii_case("/S"));

    if args.iter().any(|a| a == "--uninstall") {
        match assoc::uninstall() {
            Ok(()) => dialog(quiet, MessageLevel::Info, "dvnote has been removed."),
            Err(e) => {
                dialog(quiet, MessageLevel::Error, &e);
                std::process::exit(1);
            }
        }
        return;
    }

    let installed = assoc::installed_version();
    if !quiet && !ask(&question(installed.as_deref())) {
        return;
    }

    match install() {
        Ok(exe) if installed.is_some() => dialog(
            quiet,
            MessageLevel::Info,
            &format!("dvnote is now version {VERSION}.\n\nIf it is open, close and reopen it to use the new version.\n\n{}", exe.display()),
        ),
        Ok(exe) => {
            if quiet {
                return;
            }
            if ask("dvnote is installed.\n\nOpen Windows default apps now so you can make it the default for .md and .txt?") {
                let _ = std::process::Command::new("cmd")
                    .raw_arg(format!("/C start \"\" \"ms-settings:defaultapps?registeredAppUser={}\"", assoc::APP_NAME))
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
            }
            let _ = std::process::Command::new(exe).spawn();
        }
        Err(e) => {
            dialog(quiet, MessageLevel::Error, &e);
            std::process::exit(1);
        }
    }
}
