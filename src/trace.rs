// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
// Startup phase timing for `cargo build --release --features trace`: each mark records the time since process creation
// and the process's commit charge in memory; `flush` appends them to %TEMP%\dvnote-trace.txt. Without the feature,
// every function here compiles to nothing.

#[cfg(feature = "trace")]
mod imp {
    use std::cell::RefCell;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    thread_local! {
        static MARKS: RefCell<Vec<(u64, f64, String)>> = const { RefCell::new(Vec::new()) };
    }

    fn ft(f: FILETIME) -> u64 {
        (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
    }

    pub fn mark(label: &str) {
        let mut now = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        unsafe { GetSystemTimePreciseAsFileTime(&mut now) };
        let commit = unsafe {
            let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb);
            pmc.PrivateUsage as f64 / 1048576.0
        };
        MARKS.with(|m| m.borrow_mut().push((ft(now), commit, label.to_owned())));
    }

    pub fn flush() {
        use std::io::Write;
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut a, mut b, mut c) = (z, z, z, z);
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut a, &mut b, &mut c) };
        let start = ft(created);
        let text: String = MARKS.with(|m| {
            m.borrow_mut()
                .drain(..)
                .map(|(t, commit, label)| {
                    format!("{:8.2} ms  commit {commit:6.2} MB  {label}\n", (t - start) as f64 / 10_000.0)
                })
                .collect()
        });
        let path = std::env::temp_dir().join("dvnote-trace.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

#[cfg(feature = "trace")]
pub use imp::{flush, mark};

#[cfg(not(feature = "trace"))]
#[inline(always)]
pub fn mark(_label: &str) {}

#[cfg(not(feature = "trace"))]
#[inline(always)]
pub fn flush() {}
