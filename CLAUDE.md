# Effective Darkroom

Native Linux photo library + non-destructive RAW/JPEG editor (Lightroom-style),
in Rust: egui/eframe 0.35 (wgpu 29) UI, LibRaw via our own bindgen FFI, SQLite
catalog. Optimized for exactly two cameras: Canon EOS RP (.CR3) and Fujifilm
X100V (.RAF).

## Build / test / run

Everything runs inside the Nix dev shell (`flake.nix`; direnv via `.envrc`):

```sh
nix develop --command cargo build            # debug build
nix develop --command cargo test             # unit tests (catalog, curves, color, export naming)
nix develop --command cargo clippy           # keep this at zero warnings
nix develop --command cargo run --release
```

The desktop launcher (krunner/app menu) runs the **prebuilt**
`target/release/effective-darkroom` through `nix develop` — after code
changes run `cargo build --release` or the launcher serves a stale binary.

## Architecture

- `crates/libraw-ffi` — bindgen against **system LibRaw** (pkg-config, no
  vendoring; nixpkgs has 0.22.1). Thin safe wrapper: header-only metadata,
  embedded thumbnail extraction, full linear 16-bit decode (`user_qual=3`,
  linear gamma, camera WB, sRGB primaries). Runtime version assert in wrapper.
- `src/catalog/` — SQLite via rusqlite (bundled) at
  `~/.local/share/effective-darkroom/catalog.db`; thumbnails cached next to it.
  Images + edits (JSON params blob) + collections + flags. Migrations are
  append-only steps in `schema.rs` (bump `SCHEMA_VERSION`, add a step).
- `src/engine/` — the GPU develop pipeline (all editing is non-destructive;
  originals are never written). Passes: optional half-res YCbCr blur chain
  (texture/clarity USM + chroma NR) → main fragment pass (WB Bradford matrix →
  exposure → dehaze → sRGB encode → chroma NR → local contrast → 1024-entry
  per-channel tone LUT → vibrance/saturation) → offscreen texture shown via
  `register_native_texture`; a 256px pass is read back async for the histogram.
  The tone LUT composes: per-camera base curve (RAW only, `assets/base_curves/`)
  ∘ parametric sliders ∘ user point curves (Fritsch–Carlson monotone cubic).
  Export (`engine/export.rs`) renders full-res through this same pipeline;
  `engine/exif_out.rs` rebuilds an Exif APP1 for the encoded JPEG (source
  Exif verbatim for JPEGs, rawler's parse for RAWs, maker notes dropped).
- `src/importer/` — background import (files/folders or camera via libgphoto2),
  capture-date `YYYY/MM-DD` sorting, name+size dedupe, scan/picker streaming,
  cancel flag. Camera imports never delete from the card.
- `src/metadata_transfer.rs` — export/import of develop work as one JSON
  `.edmeta` bundle (edits, flags, collection membership) so a second machine
  can adopt it. Photos are matched by case-folded file name + byte size (the
  importer's dedupe pair), never by path; `plan()` measures a bundle against
  the catalog for confirmation before `apply()` writes, and existing local
  work is skipped or overwritten per `OnConflict`.
- `src/jobs/` — worker threads + crossbeam channels; workers clone the
  `egui::Context` and `request_repaint()`. Decode thread is latest-wins.
- `src/ui/` — one module per surface (grid, loupe, filmstrip, slideshow,
  develop panel, tone curve, dialogs, top bar). The slideshow is a
  full-screen mode (F) that replaces the whole frame — `App::ui` returns
  early, so no top bar, panels or dialogs are drawn. `src/app.rs` owns all state; `visible` is the
  filtered image list that grid/filmstrip/arrows navigate, derived from the
  active `LibrarySource` (All / LastImport / Collection) plus the flag
  filters. Last Import reads `images.import_batch`, stamped once per import
  run; batch 0 means "predates the column" and is never a run.

## Dev/test tooling (cargo examples)

- `render_cli` — render any file through the exact GUI pipeline to PNG/JPEG;
  the workhorse for headless pixel verification (`--no-base`, `--params`).
- `curve_fit` — fit per-camera base curves from (`render_cli --no-base` render,
  Adobe JPEG) pairs; paste output into `assets/base_curves/*.json`.
- `import_cli` / `export_cli` / `camera_cli` — headless import/export/camera
  smoke tests. Use `XDG_DATA_HOME=<scratch>` to isolate a test catalog.
- `metadata_cli` — headless `export` / `plan` / `apply` of an `.edmeta`
  bundle; with `XDG_DATA_HOME=<scratch>` this round-trips a real export into
  a throwaway catalog.
- `probe` (in libraw-ffi) — LibRaw metadata/thumb/decode check.
- `repaint_test`, `repaint_mode_test`, `wgpu_alloc_test`, `vram_probe` —
  repros for the machine quirks below.

Dev hooks: `ED_STARTUP_VIEW=develop`, `ED_STARTUP_IMAGE=<substring>`,
`ED_STARTUP_CROP=1`, `ED_TITLE=<caption>`.

## Machine quirks (this workstation)

- winit 0.30's Wayland backend paints one frame on this KDE session and then
  spins the main thread at 100% CPU in `wayland_backend`'s `prepare_read`,
  never redrawing → `prefer_x11()` in `main.rs` hides `WAYLAND_DISPLAY` so
  winit takes the XWayland path (winit 0.29 removed `WINIT_UNIX_BACKEND`, so
  the environment is the only lever). `ED_WAYLAND=1` forces native Wayland.
  Reproduces with a bare eframe app on every adapter, llvmpipe included —
  `examples/repaint_mode_test` bisects backend / present mode / adapter.
  Retest after a winit or KWin update.
- Backends are pinned to VULKAN|GL in `main.rs`; GL alone cannot present here
  (`incompatible_surface_backends: GL`), so this is Vulkan in practice.
- Adapter choice (`select_adapter` in `main.rs`) has to suit two machines:
  the 3060 machine, where only the discrete GPU's dmabufs are accepted
  by the compositor (the AMD iGPU fails with "importing the supplied dmabufs
  failed"), and this workstation, whose two 3090 Tis are each ~95% held by a
  vLLM server. So: surface compatibility first, then free VRAM (a discrete
  GPU with < 2 GiB free per VK_EXT_memory_budget loses), then device class,
  then the emptier card. Here that lands on the 7900X iGPU. The old "3090 Ti
  spurious OOM" was this real shortage all along. `WGPU_POWER_PREF=low`
  forces the iGPU; `examples/vram_probe` shows what each adapter can
  actually allocate. Engine allocations run under an OOM error scope
  (`oom_guard`) so a full card degrades (preview kept, blur chain skipped,
  export error) instead of panicking.
- egui embeds its fonts. Inter is bundled as primary; before using a non-ASCII
  glyph in UI strings check coverage (past tofu bug) — ⚑⚐✖✔⚠🗑📷 are safe.
- GUI verification: prefer `render_cli` headless renders. Screenshots require
  a KWin activate-by-caption script + `spectacle -a` and fail if the user is
  actively typing elsewhere (focus-stealing prevention). Synthetic keystrokes
  are not available: `wtype` finds no virtual-keyboard protocol, and xdotool's
  XTEST raises KDE's "Remote Control" consent dialog. Drive UI states through
  the config file and `ED_STARTUP_*` hooks instead.

## Conventions

- Milestone/feature commits with detailed bodies; keep clippy clean.
- Edit params are forward-compatible (`#[serde(default)]`); breaking changes
  bump `PARAMS_VERSION`.
- Approximations are documented where they diverge from Lightroom (dehaze,
  USM-based texture/clarity, no hue-twist camera profiles).
