# Cabin

A fast, pure-Rust terminal file manager with a three-pane UI, async media previews, and keyboard-first navigation.

## Status

Actively developed and fully usable for day-to-day browsing.

## Install

```powershell
cargo install --path .
```

This exposes two terminal commands:

- `cabin`
- `cab`

## Features

### Navigation
- Three-pane layout: Places, Contents, Preview
- Home directory startup with configurable start folder
- Remember last folder across sessions
- Jump directly to any path with `g`
- Parent folder navigation with `Backspace` or `Left`
- Toggle hidden files on the fly

### File Operations
- Copy, cut, paste with conflict resolution (Keep both / Replace / Cancel)
- Rename, create new files and folders
- Safe delete to Recycle Bin (cross-platform)
- Multi-select with `Space` — bulk copy, cut, and delete
- Copy current path to clipboard

### Search
- Live current-folder filter with `/`
- Recursive search with `Ctrl+F` (respects `.gitignore`, skips `node_modules` / `target` / etc.)
- `Backspace` exits search mode without navigating away

### Sorting
- Cycle sort field with `Shift+S`: Name → Size → Modified → Extension
- Directories always appear before files

### Bookmarks
- Add any directory as a named bookmark with `Ctrl+B`
- Bookmarks appear in the Places panel and persist across sessions

### Previews
- **Images** — async, multi-threaded decode (2–4 workers); pre-downscaled to terminal resolution; Triangle resampler for ~3–10× faster rendering vs. Lanczos3
- **Text files** — 500-line preview, loaded off the main thread (no UI freeze); supports `.rs`, `.toml`, `.json`, `.yaml`, `.md`, `.py`, `.ts`, `.sql`, `.sh` and 20+ more
- **PDF** — plain-text extraction from the first 10 pages via `lopdf`
- **Audio** — title, artist, album, duration, bitrate via `lofty` (MP3, FLAC, WAV, OGG, M4A, AAC)
- **Video** — duration, resolution, codec via `remeta` (MP4, MKV, WebM, MOV, AVI)
- **Folders** — child listing (up to 18 items) with file-type icons
- 80 ms debounce on cursor movement — no redundant decode jobs during fast scrolling
- All preview jobs dispatched to background threads; UI never blocks on disk I/O

### Appearance
- Four built-in themes: Dark, Light, Minimal, Mono
- Full custom theme via `#RRGGBB` / `#RGB` / named color codes
- Four border styles: Plain, Rounded, Double, Thick
- Four panel layouts: Classic, Balanced, Preview focus, Contents focus
- Multi-selected items highlighted in yellow with `*` prefix
- Header shows current directory and active sort field

### System Footer
- Live CPU, GPU (NVIDIA), RAM, swap, and disk usage
- GPU detection is lazy — `nvidia-smi` is never called on non-NVIDIA systems after the first failure
- Scrolling hint bar (toggleable)

## Controls

| Key | Action |
|-----|--------|
| `q` | Quit |
| `?` | Toggle help |
| `s` | Settings |
| `Tab` / `Shift+Tab` | Switch panel |
| `Up` / `Down` / `j` / `k` | Move selection |
| `Enter` | Open item |
| `Backspace` / `Left` | Parent folder (or exit search) |
| `h` | Toggle hidden files |
| `Space` | Toggle multi-select on item |
| `c` | Mark for copy |
| `x` | Mark for cut |
| `p` | Paste into current folder |
| `r` | Rename |
| `d` | Delete to Recycle Bin |
| `n` | New file |
| `Shift+N` | New folder |
| `y` | Copy current path to clipboard |
| `g` | Jump to path |
| `Shift+S` | Cycle sort field |
| `Ctrl+B` | Add current directory as bookmark |
| `/` | Search current folder (live filter) |
| `Ctrl+F` | Recursive search |
| `F5` | Refresh |
| `Ctrl+R` | Reset colors to defaults (in Settings) |

## Configuration

Config is stored at:

| OS | Path |
|----|------|
| Windows | `C:\Users\<user>\AppData\Roaming\Cabin\config.toml` |
| macOS | `~/Library/Application Support/Cabin/config.toml` |
| Linux | `~/.config/Cabin/config.toml` |

### Available options

```toml
theme = "dark"                  # dark | light | minimal | mono | custom
accent_color = "#00D7D7"        # #RRGGBB, #RGB, or a named color
foreground_color = "#D7D7D7"
background_color = "#000000"
muted_color = "#5F5F5F"
border_style = "rounded"        # plain | rounded | double | thick
panel_layout = "balanced"       # classic | balanced | preview_focus | contents_focus
sort_field = "name"             # name | size | modified | extension
sort_descending = false
start_dir = "home"              # home | last | /any/absolute/path
remember_last_folder = true
show_footer_tips = true
show_hidden = false

# User-defined bookmarks — also editable via Ctrl+B in the app
bookmarks = [
  ["Projects", "/home/user/projects"],
  ["Downloads", "/home/user/Downloads"],
]
```

## Notes

- Windows is the primary target; macOS and Linux are supported.
- The app runs inside your existing terminal emulator, not as a custom one.
- Image preview quality and protocol depend on terminal support (Sixel, Kitty, iTerm2). A pixel-block fallback is used when none are detected.
- Mouse capture is intentionally disabled so native terminal text selection and right-click menus continue to work.
- The Recycle Bin / Trash is used for all deletions — files are never permanently removed without an explicit bypass.
