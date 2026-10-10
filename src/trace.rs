// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
// Startup phase timing for `cargo build --release --features trace`: each mark appends the time since process creation
// and the process's commit charge to %TEMP%\dvnote-trace.txt. Without the feature, `mark` compiles to nothing.

#[cfg(feature = "trace")]
pub fn mark(label: &str) {
    use std::io::Write;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ft = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
    unsafe {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut a, mut b, mut c, mut now) = (z, z, z, z, z);
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut a, &mut b, &mut c);
        GetSystemTimePreciseAsFileTime(&mut now);
        let ms = (ft(now) - ft(created)) as f64 / 10_000.0;
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb);
        let line = format!(
            "{ms:8.2} ms  commit {:6.2} MB  ws {:6.2} MB  {label}\n",
            pmc.PrivateUsage as f64 / 1048576.0,
            pmc.WorkingSetSize as f64 / 1048576.0
        );
        let path = std::env::temp_dir().join("dvnote-trace.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[cfg(not(feature = "trace"))]
#[inline(always)]
pub fn mark(_label: &str) {}
