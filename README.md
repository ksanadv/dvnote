# dvnote

A small Notepad-style editor for `.txt` and `.md` files, written in Rust on plain Win32 with Direct2D and DirectWrite.
One 1.3 MB exe, no runtime, no admin rights. It idles at 0% CPU and a few MB of RAM, and opens a 55 MB file in about 140 ms.

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

Same machine, Windows 11 Notepad 11.2607.14.0, median of three runs, idle measured 10 s after the window settled.

| | Notepad | dvnote |
|---|---:|---:|
| start, empty window | 467 ms | 106 ms |
| start, 55 MB file | 3,095 ms | 141 ms |
| idle CPU, 55 MB file | 22.8% | 0.00% |
| memory committed, empty | 96 MB | 6 MB |
| memory committed, 55 MB file | 331 MB | 19 MB |

Large files are memory-mapped and only the lines on screen are laid out; the document is a rope, so a keystroke takes
under 0.1 ms up to 200 MB. Notepad has tabs, session restore and spell check; dvnote does not.

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
