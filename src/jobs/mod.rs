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
        image: crate::decode::full::FullImage,
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
}

fn full_decode_worker(
    rx: Receiver<FullDecodeRequest>,
    tx: Sender<JobResult>,
    ctx: egui::Context,
) {
    while let Ok(mut req) = rx.recv() {
        // Latest-wins: skip straight to the newest queued request.
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        let result = match crate::decode::full::decode_full(&req.rec) {
            Ok(image) => JobResult::FullReady {
                image_id: req.rec.id,
                generation: req.generation,
                image,
            },
            Err(e) => JobResult::FullFailed {
                image_id: req.rec.id,
                generation: req.generation,
                error: format!("{e:#}"),
            },
        };
        if tx.send(result).is_err() {
            return;
        }
        ctx.request_repaint();
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
