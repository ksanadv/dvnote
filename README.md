# dvnote

A small Notepad-style editor for `.txt` and `.md` files, written in Rust on plain Win32 and DirectWrite.
One 1.3 MB exe, no runtime, no admin rights. It idles at 0% CPU and about 2 MB of memory, and opens a 55 MB file in
about 80 ms.

## Install

Download `dvnote-<version>-windows-x64-setup.exe` from the [latest release](https://github.com/ksanadv/dvnote/releases/latest)
and run it. It installs for the current user, registers `.txt`, `.md` and `.markdown`, and adds a Start menu shortcut.
Running it again updates in place, even while dvnote is open. Silent: add `--quiet`. Uninstall from Apps & features or
**Help > Uninstall**. The `-portable.exe` is the same program without the installer.

## Features

- Open, save, drag and drop, and a prompt for unsaved changes
- Keeps the file's encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI) and line endings (CRLF/LF)
- Find / Replace, Go to line, Time/Date, word wrap, zoom, undo / redo
- Full text shaping through DirectWrite: right-to-left, CJK, emoji, font fallback
- Light, dark or system theme
- Markdown preview: `Ctrl+E` toggles it, `Ctrl+Shift+E` switches between side by side and preview only

## Compared with Windows Notepad

Same machine, Windows 11 Notepad 11.2607.14.0, idle measured 10 s after the window settled; the 55 MB file has 520,000
ASCII lines. dvnote: 900×640 window, start the median of 11 runs, memory the median of 5.

| | Notepad | dvnote |
|---|---:|---:|
| start, empty window | 467 ms | 61 ms |
| start, 55 MB file | 3,095 ms | 78 ms |
| idle CPU, 55 MB file | 22.8% | 0.00% |
| memory committed, empty | 96 MB | 2.1 MB |
| memory committed, 55 MB file | 331 MB | 4.6 MB |

Large files are memory-mapped and indexed in parallel; per-line bookkeeping stays sparse until lines are laid out, and
only the lines on screen are. Text is shaped by DirectWrite and drawn into dvnote's own bitmap, with no Direct2D or
Direct3D, and that bitmap is freed when the editor goes idle. Notepad has tabs, session restore and spell check; dvnote
does not.

## Limits

- Files up to 2 GB. Lines over 8192 characters are drawn in pieces.
- The Markdown preview is read-only, up to 1 million characters, without images.
- The in-progress IME composition is not drawn inline.

## Building and releasing

```powershell
.\build-installer.ps1             # builds the setup and portable exe into dist\
.\release.ps1 -Bump 0.4.0         # sets the version in both Cargo.toml files; commit it on a branch
.\release.ps1                     # once that is merged: tags main, and GitHub Actions publishes the release
```

Settings live in `%APPDATA%\dvnote\settings.txt`.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
