//! Fit a per-camera base curve from (our no-base render, Adobe reference)
//! image pairs.
//!
//!   cargo run --example curve_fit -- ours1.png adobe1.jpg [ours2.png adobe2.jpg ...]
//!
//! Renders should be produced with `render_cli --no-base` so the fitted curve
//! captures the full default tone difference. Prints base-curve JSON.

fn luma(px: &image::Rgb<u8>) -> f32 {
    (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32) / 255.0
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !args.is_empty() && args.len() % 2 == 0,
        "usage: curve_fit ours1 adobe1 [ours2 adobe2 ...]"
    );

    const BINS: usize = 64;
    let mut samples: Vec<Vec<f32>> = vec![Vec::new(); BINS];

    for pair in args.chunks(2) {
        let ours = image::open(&pair[0])?.to_rgb8();
        let adobe = image::open(&pair[1])?.to_rgb8();
        let adobe = image::imageops::resize(
            &adobe,
            ours.width(),
            ours.height(),
            image::imageops::FilterType::Triangle,
        );
        // Central crop to dodge framing differences from lens corrections.
        let (w, h) = (ours.width(), ours.height());
        let (x0, y0) = (w / 10, h / 10);
        let (x1, y1) = (w - w / 10, h - h / 10);
        for y in (y0..y1).step_by(3) {
            for x in (x0..x1).step_by(3) {
                let l = luma(ours.get_pixel(x, y));
                let a = luma(adobe.get_pixel(x, y));
                let bin = ((l * BINS as f32) as usize).min(BINS - 1);
                samples[bin].push(a);
            }
        }
    }

    // Median per bin.
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (i, bin) in samples.iter_mut().enumerate() {
        if bin.len() < 50 {
            continue;
        }
        bin.sort_by(f32::total_cmp);
        xs.push((i as f32 + 0.5) / BINS as f32);
        ys.push(bin[bin.len() / 2]);
    }
    anyhow::ensure!(xs.len() >= 4, "not enough tonal coverage to fit a curve");

    // Enforce monotone response (isotonic-ish: running max).
    for i in 1..ys.len() {
        if ys[i] < ys[i - 1] {
            ys[i] = ys[i - 1];
        }
    }

    // Thin to control points: endpoints + evenly spaced picks.
    const N_POINTS: usize = 9;
    let mut points: Vec<[f32; 2]> = vec![[0.0, 0.0]];
    for k in 0..N_POINTS {
        let idx = k * (xs.len() - 1) / (N_POINTS - 1);
        let (x, y) = (xs[idx], ys[idx]);
        if x > 0.01 && x < 0.99 {
            points.push([x, y.clamp(0.0, 1.0)]);
        }
    }
    points.push([1.0, 1.0]);

    println!("{{");
    println!("  \"baseline_ev\": 0.0,");
    println!("  \"points\": [");
    for (i, p) in points.iter().enumerate() {
        let comma = if i + 1 == points.len() { "" } else { "," };
        println!("    [{:.4}, {:.4}]{comma}", p[0], p[1]);
    }
    println!("  ]");
    println!("}}");
    Ok(())
}
