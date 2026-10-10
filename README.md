# dvnote

A small Notepad-style editor for `.txt` and `.md` files, written in Rust on plain Win32 with Direct2D and DirectWrite.
One 1.3 MB exe, no runtime, no admin rights. It idles at 0% CPU and a few MB of RAM, and opens a 55 MB file in about 140 ms.

## Install

Download `dvnote-Setup.exe` from the [latest release](https://github.com/ksanadv/dvnote/releases/latest) and run it.
To build it yourself instead:

```powershell
.\build-installer.ps1
.\dist\dvnote-Setup.exe
```

`dvnote-Setup.exe` (~1.8 MB) is a single self-contained file. It asks once, installs for the current user, registers
`.txt`, `.md` and `.markdown`, adds a Start menu shortcut and an Apps & features entry, and offers to open Windows default
apps so you can make dvnote the default. Windows does not allow a program to do that silently.

Running the setup again updates an existing install in place, even while dvnote is open (the running copy keeps working until you reopen it); it tells you which version it is replacing, and asks before installing an older one over a newer one. Silent install or update: `dvnote-Setup.exe --quiet`. Uninstall: Apps & features, **Help > Uninstall**, or `dvnote.exe --uninstall`.
Without the installer, `dvnote.exe --install` does the same from the app itself.

## Updates

dvnote never connects to the internet on its own. To update, choose **Help > Check for updates**, or click the small dot at
the right end of the status bar. That makes one request to github.com to read the version of the
[latest release](https://github.com/ksanadv/dvnote/releases/latest). If a newer one exists, dvnote asks before downloading
it, checks the download against the release's published SHA-256, and runs its setup silently. The open window keeps
working and the new version starts the next time dvnote is opened.

The dot turns yellow when updates have not been checked for about six months (counted from the last check, or from the
install). It only reminds; it never checks by itself. **Help > Remind me to check for updates** hides it. Nothing about
updates runs at startup, and the network libraries are only loaded when a check is made.

## Features

- Opens the file passed on the command line (that is what "Open with" does), or drag a file onto the window
- New / Open / Save / Save as, `*` in the title, and a Save / Don't save / Cancel prompt for unsaved changes
- Keeps the file's encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI) and line endings (CRLF/LF); change either from the status bar
- Find / Replace (Ctrl+F, Ctrl+H, F3, Shift+F3), Go to line (Ctrl+G), Time/Date (F5), word wrap, zoom (Ctrl +/-/0/wheel)
- Undo / redo, word-wise movement and deletion, double-click word, triple-click line
- Real text shaping through DirectWrite: Arabic joining, right-to-left and mixed-direction text, CJK, emoji, font fallback
- Light, dark or system theme (View > Theme), including the title bar, menu bar and scrollbars

### Markdown view

| key | action |
|---|---|
| `Ctrl+E` | switch between the raw text and the formatted preview |
| `Ctrl+Shift+E` | switch the preview between side by side and formatted only |

The preview shows headings, bold, italic, links, lists, code, quotes and tables. dvnote remembers the Markdown view you last
used: `.md` files reopen as raw, side by side, or formatted-only exactly as you left them, and every other file opens as
plain text. The choice is saved the moment you change it.

## Compared with Windows Notepad

Measured on the same machine against the Notepad that ships with Windows 11 (version 11.2607.14.0), started from a clean
state with a single empty tab. Each number is the median of three runs; "idle" is 10 seconds after the window settled; the
file is a 55 MB, 520,000-line text file. Scripts used are described under *How this was measured*.

| | Notepad | dvnote |
|---|---:|---:|
| **Empty window** | | |
| start until the window is shown | 467 ms | 106 ms |
| idle CPU | 0.16% | 0.00% |
| memory (private working set) | 38 MB | 3 MB |
| memory (working set) | 113 MB | 6 MB |
| memory (committed) | 96 MB | 6 MB |
| **55 MB file** | | |
| start until the file is shown | 3,095 ms | 141 ms |
| idle CPU | 22.8% | 0.00% |
| memory (private working set) | 269 MB | 4 MB |
| memory (working set) | 361 MB | 7 MB |
| memory (committed) | 331 MB | 19 MB |
| program size | Store app | 1.3 MB exe |

About the memory rows: dvnote hands its resident pages back to Windows when it is idle (after about 5 seconds without input,
when it loses focus, or when it is minimized), so the working-set figures are low and pages return on demand. The committed row
is what the process has actually allocated; for the large file that is only the line index, because the file itself is memory-mapped (see *Limits*).

What Notepad does that dvnote does not: tabs, session restore, spell check, and the extra editing features Microsoft has
been adding to the Store version. dvnote is a single-document editor (File > New window opens another one) and keeps to
plain text plus a read-only Markdown preview.

### How this was measured

- *Start until shown*: a PowerShell stopwatch from `Start-Process` until the process has a visible top-level window whose
  title is non-empty (and contains the file name when a file is opened). Notepad's session state was backed up, cleared for
  the measurement and restored afterwards.
- *Idle CPU*: processor time consumed by the process (all of Notepad's processes) over 10 seconds.
- *Memory*: working set, private working set and private bytes read from the process after the window had settled.
- Results will differ on other machines. The headless editor benchmark (`cargo test bench_ -- --ignored --nocapture`)
  measures the text engine on its own.

## Performance design

- The document is a rope ([ropey](https://crates.io/crates/ropey)): edits and line lookups are O(log n), and only the lines
  on screen are laid out, and an edit only re-lays-out the lines it touched. Headless on the DirectWrite engine, a keystroke
  takes under 0.1 ms and an arrow key 0.05 ms from 1 MB up to 200 MB.
- A UTF-8 file of 8 MB or more on a local drive is memory-mapped and indexed on several threads instead of being read: the
  text stays in the file's own pages (not committed memory) and only a line index is built. A 1 GB file opens with about
  350 MB committed instead of about 1.4 GB. The first edit or save converts it to a rope (about 0.6 s for 1 GB).
  Smaller or other files are validated, newline-normalized and turned into a rope on several threads, split at line
  boundaries: a 200 MB file decodes in about 125 ms.
- Search runs over the rope without copying it; saving streams from the rope.
- Nothing runs while idle: the caret blinks for 5 seconds after the last input and then stays solid, so there is no timer left.
- Rendering uses Direct2D's software rasterizer (4.5 ms for a full repaint of a 1600x1000 window, and about 20 MB lighter
  than the GPU-backed target). The first frame is drawn before the window is shown, and DirectWrite's fonts are warmed on a
  spare thread while the windows are created.
- The preview's data is freed when the preview is hidden, and all layout caches are capped.
- Build: static CRT, link-time optimization, one codegen unit, `opt-level = 3`, and the dialog libraries are delay-loaded.

## Source layout

| file | role |
|---|---|
| `win.rs`, `dialogs.rs`, `sys.rs` | window, menus, input, clipboard, Find/Replace and Go to dialogs (the Win32 side) |
| `draw.rs` | Direct2D canvas and DirectWrite layouts |
| `editor.rs`, `rowindex.rs`, `utf16.rs` | cursor, selection, scrolling, virtualized layout; no Windows dependencies |
| `buffer.rs`, `find.rs`, `doc.rs`, `text.rs` | rope with undo, search, file encodings and line endings, memory-mapped large files |
| `md.rs`, `preview.rs` | Markdown parsing and the preview pane |
| `settings.rs`, `assoc.rs`, `installer/` | settings file, per-user install and file associations, the setup program |
| `update.rs` | user-started update check and install against the GitHub releases |

## Limits

- Files are limited to 2 GB. Below 8 MB, or when the file is on a network/removable drive, a cloud placeholder, UTF-16,
  not valid UTF-8 or contains a lone CR, it is loaded into a rope, which peaks at about twice the file size.
- While a large file is open and unedited, dvnote holds it open for reading: other programs can read it but cannot save
  over, truncate or delete it. The first edit releases it.
- A line longer than 8192 characters is laid out in 8192-character pieces, so it is drawn broken at those boundaries.
- Pressing Enter in a file with millions of lines costs about 20 ms (the line index is rebuilt).
- The Markdown preview is only offered up to 1 million characters, and does not show images.
- Arrow keys move in logical order inside right-to-left text, not visual order.
- IME: composed text is inserted when committed; the in-progress composition is not drawn inline.
- Dark popup menus and the dark menu bar use undocumented Windows entry points (`uxtheme.dll` ordinals 135 and 136, and the
  menu-bar paint messages). If Windows stops sending them, those parts fall back to the light look; nothing else depends on them.
- The standard Find/Replace and file dialogs follow Windows, not the dvnote theme.

## Releases

```powershell
.\release.ps1 0.4.0
```

sets the version in both `Cargo.toml` files, builds and tests locally, commits, tags `v0.4.0` and pushes. The tag starts
the [Release workflow](.github/workflows/release.yml), which checks the tag against the version, runs the tests, builds the
setup program and publishes it with its SHA-256 as a GitHub release. That release is what **Check for updates** finds.

## Notes

- Settings (wrap, zoom, Markdown view, theme, window size) live in `%APPDATA%\dvnote\settings.txt`.
- Build output goes to `target/` unless a `.cargo/config.toml` outside the repo (or `CARGO_TARGET_DIR`) points it elsewhere.

## License

dvnote is free software, released under the GNU General Public License, version 3 or (at your option) any later version.
See [LICENSE](LICENSE). Every source file carries an SPDX identifier.
