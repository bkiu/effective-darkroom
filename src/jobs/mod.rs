//! Background job system. The UI thread sends requests and drains results
//! once per frame; workers hold a cloned `egui::Context` so they can wake the
//! UI when a result is ready.

use crossbeam_channel::{Receiver, Sender};

use crate::catalog::ImageRecord;
use crate::importer::ImportOutcome;

pub enum JobRequest {
    Thumb(ImageRecord),
}

/// A full-resolution decode request; `generation` implements latest-wins.
pub struct FullDecodeRequest {
    pub generation: u64,
    pub rec: ImageRecord,
}

pub enum JobResult {
    ThumbReady {
        image_id: i64,
        image: egui::ColorImage,
    },
    ThumbFailed {
        image_id: i64,
        error: String,
    },
    FullReady {
        image_id: i64,
        generation: u64,
        image: std::sync::Arc<crate::decode::full::FullImage>,
        /// False for the fast RAW preview phase; true once this is the best
        /// available decode.
        is_final: bool,
    },
    FullFailed {
        image_id: i64,
        generation: u64,
        error: String,
    },
    ImportProgress {
        current: usize,
        total: usize,
        file_name: String,
    },
    ImportDone(ImportOutcome),
    ImportScanList {
        generation: u64,
        candidates: Vec<crate::importer::ImportCandidate>,
    },
    ImportScanThumb {
        generation: u64,
        key: usize,
        image: egui::ColorImage,
    },
    ImportScanFailed {
        generation: u64,
        error: String,
    },
    ExportProgress {
        current: usize,
        total: usize,
        file_name: String,
    },
    ExportDone(crate::engine::export::ExportOutcome),
    AutoStraighten {
        image_id: i64,
        /// Suggested crop angle, or None when no confident line direction.
        angle: Option<f32>,
    },
    MoveLibraryProgress {
        current: usize,
        total: usize,
        file_name: String,
    },
    MoveLibraryDone(crate::maintenance::MoveOutcome),
}

pub struct Jobs {
    thumb_tx: Sender<JobRequest>,
    full_tx: Sender<FullDecodeRequest>,
    pub results_tx: Sender<JobResult>,
    pub results_rx: Receiver<JobResult>,
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (thumb_tx, thumb_rx) = crossbeam_channel::unbounded::<JobRequest>();
        let (results_tx, results_rx) = crossbeam_channel::unbounded::<JobResult>();

        let workers = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(2, 6))
            .unwrap_or(2);
        for i in 0..workers {
            let rx = thumb_rx.clone();
            let tx = results_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("thumb-{i}"))
                .spawn(move || thumb_worker(rx, tx, ctx))
                .expect("spawn thumb worker");
        }

        let (full_tx, full_rx) = crossbeam_channel::unbounded::<FullDecodeRequest>();
        {
            let tx = results_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("full-decode".into())
                .spawn(move || full_decode_worker(full_rx, tx, ctx))
                .expect("spawn decode worker");
        }

        Self {
            thumb_tx,
            full_tx,
            results_tx,
            results_rx,
        }
    }

    pub fn request_thumb(&self, rec: ImageRecord) {
        let _ = self.thumb_tx.send(JobRequest::Thumb(rec));
    }

    pub fn request_full_decode(&self, generation: u64, rec: ImageRecord) {
        let _ = self.full_tx.send(FullDecodeRequest { generation, rec });
    }

    /// Analyze an image for auto-straighten on a background thread.
    pub fn start_auto_straighten(&self, rec: ImageRecord, ctx: egui::Context) {
        let tx = self.results_tx.clone();
        std::thread::Builder::new()
            .name("auto-straighten".into())
            .spawn(move || {
                let angle = crate::decode::full::decode_preview(&rec)
                    .ok()
                    .and_then(|img| match img.pixels {
                        crate::decode::full::FullPixels::Srgb8(rgba) => {
                            image::RgbaImage::from_raw(img.width, img.height, rgba)
                        }
                        _ => None,
                    })
                    .and_then(|rgba| crate::engine::straighten::detect_angle(&rgba));
                let _ = tx.send(JobResult::AutoStraighten {
                    image_id: rec.id,
                    angle,
                });
                ctx.request_repaint();
            })
            .expect("spawn auto-straighten thread");
    }

    /// Spawn a one-shot export job on its own thread with its own catalog
    /// connection and a headless engine sharing the GUI's GPU device.
    pub fn start_export(
        &self,
        spec: crate::engine::export::ExportSpec,
        device: eframe::wgpu::Device,
        queue: eframe::wgpu::Queue,
        ctx: egui::Context,
    ) {
        let tx = self.results_tx.clone();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || {
                let mut engine = crate::engine::RenderEngine::new_headless(device, queue);
                let result = crate::catalog::Catalog::open_default()
                    .and_then(|catalog| {
                        crate::engine::export::run_blocking(
                            &mut engine,
                            &catalog,
                            &spec,
                            |cur, total, name| {
                                let _ = tx.send(JobResult::ExportProgress {
                                    current: cur,
                                    total,
                                    file_name: name.to_string(),
                                });
                                ctx.request_repaint();
                            },
                        )
                    });
                let outcome = result.unwrap_or_else(|e| {
                    crate::engine::export::ExportOutcome {
                        exported: 0,
                        errors: vec![format!("export failed: {e:#}")],
                    }
                });
                let _ = tx.send(JobResult::ExportDone(outcome));
                ctx.request_repaint();
            })
            .expect("spawn export thread");
    }
}

fn full_decode_worker(
    rx: Receiver<FullDecodeRequest>,
    tx: Sender<JobResult>,
    ctx: egui::Context,
) {
    use crate::catalog::FileKind;
    use crate::decode::full;
    use std::sync::Arc;

    // Small cache of final decodes so stepping between images is instant.
    let mut cache: lru::LruCache<i64, Arc<full::FullImage>> =
        lru::LruCache::new(std::num::NonZeroUsize::new(2).unwrap());
    let mut carried: Option<FullDecodeRequest> = None;

    loop {
        let mut req = match carried.take() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            },
        };
        // Latest-wins: skip straight to the newest queued request.
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }

        let send = |result: JobResult| -> bool {
            let ok = tx.send(result).is_ok();
            ctx.request_repaint();
            ok
        };

        if let Some(image) = cache.get(&req.rec.id) {
            if !send(JobResult::FullReady {
                image_id: req.rec.id,
                generation: req.generation,
                image: image.clone(),
                is_final: true,
            }) {
                return;
            }
            continue;
        }

        match req.rec.kind {
            FileKind::Jpeg => {
                let result = match full::decode_preview(&req.rec) {
                    Ok(image) => {
                        let image = Arc::new(image);
                        cache.put(req.rec.id, image.clone());
                        JobResult::FullReady {
                            image_id: req.rec.id,
                            generation: req.generation,
                            image,
                            is_final: true,
                        }
                    }
                    Err(e) => JobResult::FullFailed {
                        image_id: req.rec.id,
                        generation: req.generation,
                        error: format!("{e:#}"),
                    },
                };
                if !send(result) {
                    return;
                }
            }
            FileKind::Raw => {
                // Phase 1: instant embedded preview.
                if let Ok(preview) = full::decode_preview(&req.rec) {
                    if !send(JobResult::FullReady {
                        image_id: req.rec.id,
                        generation: req.generation,
                        image: Arc::new(preview),
                        is_final: false,
                    }) {
                        return;
                    }
                }
                // A newer request may have arrived while extracting the
                // preview; if so skip the expensive linear decode.
                if let Ok(newer) = rx.try_recv() {
                    carried = Some(newer);
                    continue;
                }
                // Phase 2: real linear decode.
                let result = match full::decode_linear_raw(&req.rec) {
                    Ok(image) => {
                        let image = Arc::new(image);
                        cache.put(req.rec.id, image.clone());
                        JobResult::FullReady {
                            image_id: req.rec.id,
                            generation: req.generation,
                            image,
                            is_final: true,
                        }
                    }
                    Err(e) => JobResult::FullFailed {
                        image_id: req.rec.id,
                        generation: req.generation,
                        error: format!("{e:#}"),
                    },
                };
                if !send(result) {
                    return;
                }
            }
        }
    }
}

fn thumb_worker(rx: Receiver<JobRequest>, tx: Sender<JobResult>, ctx: egui::Context) {
    while let Ok(JobRequest::Thumb(rec)) = rx.recv() {
        let result = match crate::decode::thumbs::get_or_create(&rec) {
            Ok(rgba) => {
                let size = [rgba.width() as usize, rgba.height() as usize];
                JobResult::ThumbReady {
                    image_id: rec.id,
                    image: egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                }
            }
            Err(e) => JobResult::ThumbFailed {
                image_id: rec.id,
                error: format!("{e:#}"),
            },
        };
        if tx.send(result).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}
