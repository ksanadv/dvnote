// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
// Update checks against the official GitHub releases. Nothing here runs unless the user asks for a check; the work is
// done on a spare thread and winhttp.dll is delay-loaded, so a normal start never touches the network or loads it.
use crate::sys::wide;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Networking::WinHttp::*;

pub const REPO: &str = "ksanadv/dvnote";
const HOST: &str = "github.com";
const VERSION: &str = env!("CARGO_PKG_VERSION");
// the status bar reminder turns yellow when nothing has been checked for about six months
const REMIND_AFTER: Duration = Duration::from_secs(182 * 24 * 3600);
const MAX_DOWNLOAD: usize = 64 << 20;

static BUSY: AtomicBool = AtomicBool::new(false);

pub enum Latest {
    Current,
    Newer(String),
}

fn stamp_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(crate::assoc::APP_NAME).join("last-update-check.txt"))
}

// kept apart from settings.txt, which every open window rewrites from its own copy when it closes
fn record_check() {
    let Some(p) = stamp_path() else { return };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, now.to_string());
}

// counted from the last check, or from when this exe was installed or updated if there was none
pub fn reminder_due() -> bool {
    let checked = stamp_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|s| UNIX_EPOCH + Duration::from_secs(s));
    let installed = std::env::current_exe().and_then(std::fs::metadata).and_then(|m| m.modified()).ok();
    let Some(since) = checked.max(installed) else { return false };
    SystemTime::now().duration_since(since).is_ok_and(|d| d > REMIND_AFTER)
}

// runs `work` on a spare thread unless a check or download is already in progress
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + Send + 'static,
) -> bool {
    if BUSY.swap(true, Ordering::AcqRel) {
        return false;
    }
    std::thread::spawn(move || {
        let r = work();
        BUSY.store(false, Ordering::Release);
        done(r);
    });
    true
}

// one request with redirects off: github.com answers /releases/latest with a redirect to /releases/tag/<tag>
pub fn latest() -> Result<Latest, String> {
    let r = Request::get(&format!("/{REPO}/releases/latest"), false)?;
    let tag = r.header(WINHTTP_QUERY_LOCATION).and_then(|l| version_from_location(&l));
    let Some(version) = tag else {
        return match r.status() {
            Some(404) | Some(200) | Some(300..=399) => {
                Err(format!("No release has been published at {HOST}/{REPO} yet."))
            }
            Some(s) => Err(format!("{HOST} answered with HTTP status {s}.")),
            None => Err(format!("{HOST} sent no answer.")),
        };
    };
    record_check();
    Ok(if crate::assoc::is_newer(&version, VERSION) { Latest::Newer(version) } else { Latest::Current })
}

// downloads the release's setup program, checks it against the published SHA-256 and runs it silently; the running copy
// keeps working and the new version starts the next time dvnote is opened
pub fn install(version: &str) -> Result<(), String> {
    let name = setup_name(version);
    let base = format!("/{REPO}/releases/download/v{version}/{name}");
    let setup = Request::get(&base, true)?.body("the update")?;
    let sums = Request::get(&format!("{base}.sha256"), true)?.body("the update's checksum")?;
    let expected = parse_sha256(&String::from_utf8_lossy(&sums)).ok_or("The update's checksum file is not valid.")?;
    if sha256(&setup) != Some(expected) {
        return Err("The downloaded update does not match its published checksum, so it was not installed.".into());
    }
    let dir = std::env::temp_dir().join("dvnote-update");
    let exe = dir.join(&name);
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&exe, &setup))
        .map_err(|e| format!("Cannot save the update: {e}"))?;
    let status = std::process::Command::new(&exe).arg("--quiet").status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("The setup program failed ({s}).")),
        Err(e) => Err(format!("Cannot run the setup program: {e}")),
    }
}

// the release asset name; build-installer.ps1 and the Release workflow use the same pattern
fn setup_name(version: &str) -> String {
    format!("dvnote-{version}-windows-x64-setup.exe")
}

fn version_from_location(location: &str) -> Option<String> {
    let tag = location.rsplit_once("/releases/tag/")?.1;
    let v = tag.strip_prefix('v').unwrap_or(tag);
    let valid = !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| v.to_string())
}

// the first token of a `sha256sum`-style line
fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let hex = text.trim_start_matches('\u{feff}').split_whitespace().next()?.as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    use windows_sys::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    let len = u32::try_from(data.len()).ok()?;
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, data.as_ptr(), len, out.as_mut_ptr(), 32) };
    (status == 0).then_some(out)
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

// fields drop in order: request, connection, session
struct Request {
    req: Handle,
    _connect: Handle,
    _session: Handle,
}

fn failed(what: &str) -> String {
    format!("Could not {what} {HOST} ({}). Check the internet connection.", std::io::Error::last_os_error())
}

impl Request {
    fn get(path: &str, follow_redirects: bool) -> Result<Request, String> {
        let agent = wide(&format!("dvnote/{VERSION}"));
        let (host, path) = (wide(HOST), wide(path));
        unsafe {
            let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
            if session.0.is_null() {
                return Err(failed("reach"));
            }
            WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
            let connect = Handle(WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
            if connect.0.is_null() {
                return Err(failed("reach"));
            }
            let verb = wide("GET");
            let req = Handle(WinHttpOpenRequest(
                connect.0,
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ));
            if req.0.is_null() {
                return Err(failed("reach"));
            }
            let policy = if follow_redirects {
                WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP
            } else {
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER
            };
            WinHttpSetOption(req.0, WINHTTP_OPTION_REDIRECT_POLICY, &policy as *const u32 as *const c_void, 4);
            if WinHttpSendRequest(req.0, null(), 0, null(), 0, 0, 0) == 0
                || WinHttpReceiveResponse(req.0, null_mut()) == 0
            {
                return Err(failed("connect to"));
            }
            Ok(Request { req, _connect: connect, _session: session })
        }
    }

    fn status(&self) -> Option<u32> {
        let (mut code, mut len) = (0u32, 4u32);
        let flags = WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER;
        let ok = unsafe {
            WinHttpQueryHeaders(self.req.0, flags, null(), &mut code as *mut u32 as *mut c_void, &mut len, null_mut())
        };
        (ok != 0).then_some(code)
    }

    fn header(&self, which: u32) -> Option<String> {
        let mut buf = [0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let ok = unsafe {
            WinHttpQueryHeaders(self.req.0, which, null(), buf.as_mut_ptr() as *mut c_void, &mut len, null_mut())
        };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize / 2]))
    }

    fn body(self, what: &str) -> Result<Vec<u8>, String> {
        match self.status() {
            Some(200) => {}
            Some(s) => return Err(format!("Could not download {what}: {HOST} answered with HTTP status {s}.")),
            None => return Err(failed("download from")),
        }
        let mut out = Vec::new();
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let mut read = 0u32;
            let ok = unsafe {
                WinHttpReadData(self.req.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read)
            };
            if ok == 0 {
                return Err(failed("download from"));
            }
            if read == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&chunk[..read as usize]);
            if out.len() > MAX_DOWNLOAD {
                return Err(format!("{what} is larger than expected, so it was not installed."));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_to_version() {
        let v = |s: &str| version_from_location(s);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v0.4.1").as_deref(), Some("0.4.1"));
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/1.2").as_deref(), Some("1.2"));
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v1.0-beta"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v1..2"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v"), None);
    }

    #[test]
    fn checksum_file() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let empty = sha256(b"").unwrap();
        assert_eq!(parse_sha256(&format!("{hex}  dvnote-1.2.3-windows-x64-setup.exe\n")), Some(empty));
        assert_eq!(parse_sha256(&hex.to_uppercase()), Some(empty));
        assert_eq!(parse_sha256(&format!("\u{feff}{hex}  dvnote-1.2.3-windows-x64-setup.exe")), Some(empty));
        assert_eq!(parse_sha256("abc  dvnote-1.2.3-windows-x64-setup.exe"), None);
        assert_eq!(parse_sha256(""), None);
    }
}
