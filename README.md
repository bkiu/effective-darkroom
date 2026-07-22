# Effective Darkroom

A native Linux photo library and non-destructive RAW/JPEG editor in the
spirit of Lightroom, written in Rust (egui/eframe + wgpu). Optimized for the
Canon EOS RP (.CR3) and Fujifilm X100V (.RAF).

## Running

```sh
nix develop        # or rely on direnv (.envrc)
cargo run --release
```

The dev shell provides the Rust toolchain, system LibRaw, bindgen deps and
the Wayland/Vulkan runtime libraries.

## Features

- **Import** RAW/JPEG from files, folders, or a **USB-connected camera**
  (PTP via libgphoto2; tested with the EOS RP). Photos are organized into
  `YYYY/MM-DD` folders by capture date. Copy or move for file imports;
  camera imports download only photos not already in the library and never
  delete from the card. On first start the app asks where the library should
  live; that folder is the default import destination from then on.
- **Library**: virtualized thumbnail grid (click / ctrl / shift selection),
  loupe view with fit & 1:1 zoom and panning. Keys: `G` grid, `E` loupe,
  `D` develop, arrow keys to step between photos.
- **Develop** (upper-right tab): White balance (Temp/Tint, As Shot for RAW),
  Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Texture, Clarity,
  Dehaze, Vibrance, Saturation, plus a tone curve editor with draggable
  points, per-channel (RGB) curves and a live histogram. All edits are
  non-destructive: originals are never modified; parameters live in the
  catalog and re-render on the GPU in real time.
- **RAW pipeline**: LibRaw decode (AHD for Bayer, 3-pass Markesteijn for
  X-Trans) to linear 16-bit, camera As-Shot white balance, per-camera base
  tone curve fitted against Adobe reference renders. RAWs open instantly via
  their embedded preview while the full decode runs in the background.
- **Export**: batch export the grid selection with shared settings — JPEG
  quality, long-edge resize, optional filename prefix, original or
  `YYYYMMDD_HHMMSS` naming. Exports render through the exact same GPU
  pipeline as the preview.
- **Copy/paste develop settings** between photos (buttons in Develop; grid
  context menu pastes to the whole selection).

## Storage

- Catalog (SQLite): `~/.local/share/effective-darkroom/catalog.db`
- Thumbnail cache: `~/.local/share/effective-darkroom/thumbs/`
- App config: `~/.config/effective-darkroom/config.json`

## Development tools (cargo examples)

- `import_cli` — headless import (respects `XDG_DATA_HOME` for a scratch catalog)
- `export_cli` — headless batch export of the whole catalog
- `render_cli` — render one file through the develop engine to PNG/JPEG
- `curve_fit` — fit a per-camera base curve from (our `--no-base` render,
  Adobe JPEG) pairs; output goes into `assets/base_curves/`
- `probe` (in `crates/libraw-ffi`) — LibRaw metadata/thumb/decode smoke test

## Machine-specific notes

- The wgpu backend is pinned to Vulkan/GL because automatic selection stalls
  the winit event loop on KDE Wayland after the first frame.
- The adapter defaults to the low-power (integrated) GPU: the NVIDIA
  driver on the dev machine fails large allocations with spurious OOM
  (see `examples/wgpu_alloc_test.rs`). Override with `WGPU_POWER_PREF=high`.
