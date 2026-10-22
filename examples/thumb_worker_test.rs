//! Headless check of the thumbnail job pipeline (workers + channels).

use effective_darkroom::catalog::Catalog;
use effective_darkroom::jobs::{JobResult, Jobs};

fn main() -> anyhow::Result<()> {
    let ctx = egui::Context::default();
    let jobs = Jobs::new(ctx, None);
    let catalog = Catalog::open_default()?;
    let images = catalog.all_images()?;
    println!("requesting {} thumbs", images.len());
    let n = images.len();
    for rec in images {
        jobs.request_thumb(rec);
    }
    let mut got = 0;
    while got < n {
        match jobs
            .results_rx
            .recv_timeout(std::time::Duration::from_secs(10))
        {
            Ok(JobResult::ThumbReady { image_id, image }) => {
                println!("ready {image_id}: {}x{}", image.size[0], image.size[1]);
                got += 1;
            }
            Ok(JobResult::ThumbFailed { image_id, error }) => {
                println!("FAILED {image_id}: {error}");
                got += 1;
            }
            Ok(_) => {}
            Err(e) => {
                println!("timeout/disconnect: {e}");
                break;
            }
        }
    }
    Ok(())
}
