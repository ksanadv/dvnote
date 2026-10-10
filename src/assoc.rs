// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

pub const APP_NAME: &str = "dvnote";
const PROG_ID: &str = "dvnote.Document";
pub const EXE_NAME: &str = "dvnote.exe";
const EXTENSIONS: [&str; 3] = [".txt", ".md", ".markdown"];
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Programs").join(APP_NAME)
}

fn shortcut_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs").join(format!("{APP_NAME}.lnk")))
}

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
}

pub fn is_installed() -> bool {
    let Ok(key) = hkcu().open_subkey_with_flags(format!(r"Software\Classes\{PROG_ID}\shell\open\command"), KEY_READ)
    else {
        return false;
    };
    let cmd: String = key.get_value("").unwrap_or_default();
    let exe = cmd.trim_start_matches('"').split('"').next().unwrap_or("");
    !exe.is_empty() && Path::new(exe).is_file()
}

#[allow(dead_code)] // used by the installer, which shares this file
pub fn installed_version() -> Option<String> {
    hkcu()
        .open_subkey_with_flags(format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{APP_NAME}"), KEY_READ)
        .ok()?
        .get_value("DisplayVersion")
        .ok()
}

#[allow(dead_code)]
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

// A running exe cannot be overwritten but it can be renamed, so the old file is moved aside and the new one written in
// its place. The old copy is deleted right away unless it is still running; the next update removes it then.
pub fn replace_exe(target: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<(), String> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    let moved = target.exists();
    if moved {
        std::fs::rename(target, &old).map_err(|e| format!("Cannot replace {}: {e}", target.display()))?;
    }
    if let Err(e) = write(target) {
        if moved {
            let _ = std::fs::rename(&old, target);
        }
        return Err(format!("Cannot write {}: {e}", target.display()));
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

pub fn install() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    let target = dir.join(EXE_NAME);
    let same = std::fs::canonicalize(&current).ok() == std::fs::canonicalize(&target).ok();
    if !same {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        replace_exe(&target, |t| std::fs::copy(&current, t).map(|_| ()))?;
    }
    finish_install(&target)?;
    Ok(target)
}

pub fn finish_install(exe: &Path) -> Result<(), String> {
    register(exe).map_err(|e| format!("Registry write failed: {e}"))?;
    create_shortcut(exe);
    notify_shell();
    Ok(())
}

fn register(exe: &Path) -> std::io::Result<()> {
    let exe = exe.display().to_string();
    let command = format!("\"{exe}\" \"%1\"");
    let root = hkcu();

    let (class, _) = root.create_subkey(format!(r"Software\Classes\{PROG_ID}"))?;
    class.set_value("", &"Text or Markdown document")?;
    class.create_subkey("DefaultIcon")?.0.set_value("", &format!("\"{exe}\",0"))?;
    class.create_subkey(r"shell\open\command")?.0.set_value("", &command)?;

    let (app, _) = root.create_subkey(format!(r"Software\Classes\Applications\{EXE_NAME}"))?;
    app.set_value("FriendlyAppName", &APP_NAME)?;
    app.create_subkey(r"shell\open\command")?.0.set_value("", &command)?;
    let (types, _) = app.create_subkey("SupportedTypes")?;

    let (caps, _) = root.create_subkey(format!(r"Software\{APP_NAME}\Capabilities"))?;
    caps.set_value("ApplicationName", &APP_NAME)?;
    caps.set_value("ApplicationDescription", &"Simple editor for text and Markdown files")?;
    let (assoc, _) = caps.create_subkey("FileAssociations")?;
    for ext in EXTENSIONS {
        assoc.set_value(ext, &PROG_ID)?;
        types.set_value(ext, &"")?;
        let (with, _) = root.create_subkey(format!(r"Software\Classes\{ext}\OpenWithProgids"))?;
        with.set_raw_value(PROG_ID, &winreg::RegValue { vtype: winreg::enums::REG_NONE, bytes: Vec::new().into() })?;
    }
    root.create_subkey(r"Software\RegisteredApplications")?
        .0
        .set_value(APP_NAME, &format!(r"Software\{APP_NAME}\Capabilities"))?;

    let (un, _) = root.create_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{APP_NAME}"))?;
    un.set_value("DisplayName", &APP_NAME)?;
    un.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
    un.set_value("Publisher", &APP_NAME)?;
    un.set_value("DisplayIcon", &exe)?;
    un.set_value("InstallLocation", &install_dir().display().to_string())?;
    un.set_value("UninstallString", &format!("\"{exe}\" --uninstall"))?;
    un.set_value("NoModify", &1u32)?;
    un.set_value("NoRepair", &1u32)?;
    Ok(())
}

fn create_shortcut(exe: &Path) {
    let Some(lnk) = shortcut_path() else { return };
    let quote = |p: &str| p.replace('\'', "''");
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.IconLocation='{},0';$s.Save()",
        quote(&lnk.display().to_string()),
        quote(&exe.display().to_string()),
        quote(&exe.display().to_string())
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub fn uninstall() -> Result<(), String> {
    let root = hkcu();
    for ext in EXTENSIONS {
        if let Ok(with) = root
            .open_subkey_with_flags(format!(r"Software\Classes\{ext}\OpenWithProgids"), winreg::enums::KEY_SET_VALUE)
        {
            let _ = with.delete_value(PROG_ID);
        }
    }
    for key in [
        format!(r"Software\Classes\{PROG_ID}"),
        format!(r"Software\Classes\Applications\{EXE_NAME}"),
        format!(r"Software\{APP_NAME}"),
        format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{APP_NAME}"),
    ] {
        let _ = root.delete_subkey_all(key);
    }
    if let Ok(k) = root.open_subkey_with_flags(r"Software\RegisteredApplications", winreg::enums::KEY_SET_VALUE) {
        let _ = k.delete_value(APP_NAME);
    }
    if let Some(lnk) = shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    notify_shell();

    let dir = install_dir();
    if dir.join(EXE_NAME).is_file() {
        let cmd = format!("/C ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display());
        std::process::Command::new("cmd")
            // cmd.exe needs raw quotes
            .raw_arg(&cmd)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Unregistered, but could not remove {}: {e}", dir.display()))?;
    }
    Ok(())
}
