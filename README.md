# Effective Darkroom

A native Linux photo library and non-destructive RAW/JPEG editor in the
spirit of Lightroom, written in Rust (egui/eframe + wgpu). Optimized for the
Canon EOS RP (.CR3) and Fujifilm X100V (.RAF).

## Running

```sh
nix develop        # or rely on direnv (.envrc)
cargo run --release
```

The dev shell provides the Rust toolchain, system LibRaw, libgphoto2,
bindgen deps and the Wayland/Vulkan runtime libraries. A user desktop entry
("Effective Darkroom" in the app menu / krunner) launches the prebuilt
release binary.

## Features

- **Import** from files, folders, or a USB-connected camera (PTP via
  libgphoto2). A picker shows thumbnail previews of everything available
  with check/uncheck, "New only" preselection, and duplicate detection;
  imports show progress and can be stopped mid-way. Photos are organized
  into `YYYY/MM-DD` folders by capture date (copy or move for file imports;
  camera imports never touch the card). On first start the app asks where
  the library should live and uses it as the default destination.
- **Library**: thumbnail grid with multi-selection, loupe with fit/1:1 zoom
  and panning, filmstrip and a flag/delete toolbar in the single-image
  views, EXIF info overlay (shutter, aperture, ISO, focal length, …).
- **Collections & flags**: named collections in the left panel; pick/reject
  flags with distinct badges (rejects render dimmed); a Show filter row
  narrows any view by flag status and drives grid, filmstrip and arrow-key
  navigation alike.
- **Develop** (upper-right tab): Crop & straighten (interactive overlay
  with aspect ratios, angle slider, and Lightroom-style Auto straighten
  that detects dominant lines); White balance (Temp/Tint, As Shot for
  RAW), Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Texture,
  Clarity, Dehaze, Vibrance, Saturation, chroma noise reduction, and a tone
  curve editor with draggable points, per-channel (RGB) curves and a live
  histogram. All edits are non-destructive: originals are never modified;
  parameters live in the central catalog and re-render on the GPU in real
  time. Copy/paste settings between photos (grid context menu pastes to the
  whole selection).
- **RAW pipeline**: LibRaw decode (AHD for Bayer, 3-pass Markesteijn for
  X-Trans) to linear 16-bit, As-Shot white balance, per-camera base tone
  curves fitted against Adobe reference renders, default color noise
  reduction. RAWs open instantly via their embedded preview while the full
  decode runs in the background.
- **Export**: Lightroom-style dialog — JPEG (through the develop pipeline)
  or DNG — either a lossless conversion of the untouched raw data or
  a linear DNG with the develop edits baked into 16-bit pixels (EXIF
  carried over). Exported JPEGs keep the original capture time, camera,
  lens and exposure metadata, with orientation and dimensions corrected to
  match the pixels actually written; base location (Pictures / Documents /
  Nextcloud / custom) with optional subfolder, JPEG quality, long-edge
  resize (default 3000 px), filename prefix, original or `YYYYMMDD_HHMMSS`
  naming. Batch-exports the selection through the exact same GPU pipeline
  as the preview; settings persist between sessions.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `G` / `Q` | Grid view |
| `E` | Loupe view |
| `D` | Develop |
| `←` `→` | Previous / next photo |
| `P` / `U` / `X` | Pick / unflag / reject |
| `I` | EXIF info overlay |
| `Delete` | Remove from library / move to trash |
| `Ctrl` `A` | Select every photo in the current view |
| `F6` | Show / hide the filmstrip |
| `Z` | Photo zoom: toggle fit and 1:1 (double-click does the same) |
| `+` / `-` / `0` | Photo zoom in / out / fit |
| `Ctrl` `+`/`-`/`0` | UI zoom |

In the loupe and develop views the mouse wheel zooms the photo around the
pointer, and dragging pans once it no longer fits the window.

## Storage

- Catalog (SQLite): `~/.local/share/effective-darkroom/catalog.db`
- Thumbnail cache: `~/.local/share/effective-darkroom/thumbs/`
- App config: `~/.config/effective-darkroom/config.json`

The Settings screen (gear icon) can change the default library location,
relocate the whole library (files are moved and the catalog re-pointed),
and move the catalog + thumbnail cache to another folder.

## Development tools (cargo examples)

- `render_cli` — render one file through the develop engine to PNG/JPEG
- `curve_fit` — fit a per-camera base curve from (our `--no-base` render,
  Adobe JPEG) pairs; output goes into `assets/base_curves/`
- `import_cli` / `export_cli` / `camera_cli` — headless import/export/camera
  tests (respect `XDG_DATA_HOME` for a scratch catalog)
- `probe` (in `crates/libraw-ffi`) — LibRaw metadata/thumb/decode smoke test

## Design

The UI follows the design system in `docs/design-system.html` (v2, dark):
oklch-authored tokens, IBM Plex Sans for interface text, IBM Plex Mono for
all numeric readouts, bipolar center-fill adjustment sliders, and custom
window chrome. Reference `src/ui/theme.rs` tokens in code — never hardcode
colors.

## Credits

- UI fonts: [IBM Plex](https://www.ibm.com/plex/) Sans & Mono (OFL 1.1),
  bundled in `assets/fonts/`.

## Machine-specific notes

- The wgpu backend is pinned to Vulkan/GL because automatic selection stalls
  the winit event loop on KDE Wayland after the first frame.
- The adapter defaults to the low-power (integrated) GPU: the NVIDIA
  driver on the dev machine fails large allocations with spurious OOM
  (see `examples/wgpu_alloc_test.rs`). Override with `WGPU_POWER_PREF=high`.
