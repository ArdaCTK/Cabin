# Cabin

A fast, simple, pure Rust terminal file manager with a three-pane UI, media previews, and keyboard-first navigation.

## Status

Early development, but fully usable for day-to-day browsing.

## Install

```powershell
cargo install --path .
```

This exposes two terminal commands:

- `cabin`
- `cab`

## Features

- Three-pane layout: Places, Contents, Preview
- Home directory startup with configurable start folder
- Folder navigation with keyboard shortcuts
- File icons by type for quicker scanning
- Image previews inside supported terminals
- Text previews for common code and document files
- Audio and video metadata previews
- Copy, cut, paste, rename, create, and safe delete
- Conflict handling for copy and move operations
- Recursive search and current-folder filtering
- Configurable theme, colors, border style, and panel layout
- Live footer with system performance stats
- Terminal state restore on exit

## Controls

- `q` quit
- `?` help
- `s` settings
- `Ctrl+R` reset colors to defaults in Settings
- `Tab` switch panel
- `Enter` open item
- `Backspace` parent folder
- `h` toggle hidden files
- `c` copy, `x` cut, `p` paste
- `r` rename
- `d` delete to Recycle Bin
- `n` new file
- `Shift+n` new folder
- `y` copy path
- `/` search current folder
- `Ctrl+f` recursive search
- `F5` refresh

## Configuration

Cabin stores its config in:

`C:\Users\<user>\AppData\Roaming\Cabin\config.toml`

Settings cover theme, custom colors, border style, panel layout, start folder, last-folder memory, footer tips, and hidden-file visibility.

## Notes

- Windows is the primary target today.
- The app runs inside your existing terminal, not as a custom terminal emulator.
- Image preview quality depends on terminal support, with fallback rendering when needed.
