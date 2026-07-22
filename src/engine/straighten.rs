//! Auto-straighten: estimate the rotation that levels the image, from the
//! dominant near-axis edge orientation (Sobel gradients folded modulo 90°,
//! magnitude-weighted histogram — the classic auto-level approach).

/// Estimated correction angle in degrees (apply as the crop angle), or None
/// when no confident dominant line direction exists (e.g. portraits).
pub fn detect_angle(img: &image::RgbaImage) -> Option<f32> {
    // Work on a small grayscale copy.
    const ANALYZE_EDGE: u32 = 1200;
    let scaled = if img.width().max(img.height()) > ANALYZE_EDGE {
        image::imageops::thumbnail(
            img,
            (img.width() * ANALYZE_EDGE / img.width().max(img.height())).max(1),
            (img.height() * ANALYZE_EDGE / img.width().max(img.height())).max(1),
        )
    } else {
        img.clone()
    };
    let (w, h) = (scaled.width() as usize, scaled.height() as usize);
    if w < 32 || h < 32 {
        return None;
    }
    let gray: Vec<f32> = scaled
        .pixels()
        .map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32)
        .collect();

    // Sobel gradients; histogram of edge orientations folded into
    // [-45°, +45°) around the nearest axis, weighted by magnitude.
    const MAX_DEV: f32 = 15.0; // only consider lines within ±15° of an axis
    const BIN_PER_DEG: f32 = 4.0; // 0.25° bins
    let bins = (2.0 * MAX_DEV * BIN_PER_DEG) as usize;
    let mut hist = vec![0.0f64; bins];
    let mut total_weight = 0.0f64;

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let px = |dx: isize, dy: isize| {
                gray[(y as isize + dy) as usize * w + (x as isize + dx) as usize]
            };
            let gx = (px(1, -1) + 2.0 * px(1, 0) + px(1, 1))
                - (px(-1, -1) + 2.0 * px(-1, 0) + px(-1, 1));
            let gy = (px(-1, 1) + 2.0 * px(0, 1) + px(1, 1))
                - (px(-1, -1) + 2.0 * px(0, -1) + px(1, -1));
            let mag = (gx * gx + gy * gy).sqrt();
            if mag < 80.0 {
                continue;
            }
            total_weight += mag as f64;
            // Edge direction is perpendicular to the gradient; folding the
            // gradient angle modulo 90° treats horizontals and verticals
            // alike. Positive dev = lines rotated clockwise on screen.
            let grad_deg = gy.atan2(gx).to_degrees();
            let mut dev = grad_deg.rem_euclid(90.0);
            if dev >= 45.0 {
                dev -= 90.0;
            }
            if dev.abs() >= MAX_DEV {
                continue;
            }
            let bin = (((dev + MAX_DEV) * BIN_PER_DEG) as usize).min(bins - 1);
            hist[bin] += mag as f64;
        }
    }
    if total_weight <= 0.0 {
        return None;
    }

    // Smooth lightly, then take the strongest peak.
    let smoothed: Vec<f64> = (0..bins)
        .map(|i| {
            let mut acc = 0.0;
            for (di, k) in [(-2i32, 0.1), (-1, 0.25), (0, 0.3), (1, 0.25), (2, 0.1)] {
                let j = i as i32 + di;
                if (0..bins as i32).contains(&j) {
                    acc += hist[j as usize] * k;
                }
            }
            acc
        })
        .collect();
    let (peak_bin, &peak) = smoothed
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;

    // Confidence: some near-axis energy must exist at all, and the peak must
    // stand clearly above the histogram's average level (a flat histogram
    // means no dominant line direction — e.g. portraits, foliage).
    let near_axis: f64 = hist.iter().sum();
    let mean = smoothed.iter().sum::<f64>() / bins as f64;
    tracing::debug!(
        "auto-straighten: near-axis share {:.3}, peak/mean {:.2}",
        near_axis / total_weight,
        peak / mean.max(1e-9),
    );
    // Calibrated against real photos: architecture scores ~2.1+, flat
    // texture/portraits ~1.1.
    if near_axis < total_weight * 0.05 || peak < mean * 1.8 {
        return None;
    }

    // Sub-bin refinement via center of mass around the peak.
    let lo = peak_bin.saturating_sub(3);
    let hi = (peak_bin + 3).min(bins - 1);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (i, v) in smoothed.iter().enumerate().take(hi + 1).skip(lo) {
        num += v * i as f64;
        den += v;
    }
    let refined_bin = if den > 0.0 { num / den } else { peak_bin as f64 };
    let dev = refined_bin as f32 / BIN_PER_DEG - MAX_DEV;

    if dev.abs() < 0.05 {
        return Some(0.0);
    }
    // Rotating by -dev brings the dominant lines back to the axis.
    Some((-dev).clamp(-15.0, 15.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grid pattern rotated by `angle` degrees, rendered analytically.
    fn rotated_grid(angle_deg: f32) -> image::RgbaImage {
        let (w, h) = (600u32, 400u32);
        let rad = angle_deg.to_radians();
        let (s, c) = (rad.sin(), rad.cos());
        image::RgbaImage::from_fn(w, h, |x, y| {
            // Inverse-rotate the pixel position; positive angle = the
            // pattern appears rotated clockwise on screen (y down).
            let dx = x as f32 - w as f32 / 2.0;
            let dy = y as f32 - h as f32 / 2.0;
            let u = c * dx + s * dy;
            let v = -s * dx + c * dy;
            // Soft-edged lines (hard binary lines alias into pixel
            // staircases whose local gradients are axis-aligned).
            let dist_u = (u.rem_euclid(60.0) - 30.0).abs(); // 30 at line center
            let dist_v = (v.rem_euclid(60.0) - 30.0).abs();
            let line = |d: f32| ((d - 26.0) / 4.0).clamp(0.0, 1.0); // 1 on line
            let strength = line(dist_u).max(line(dist_v));
            let val = (235.0 - 215.0 * strength) as u8;
            image::Rgba([val, val, val, 255])
        })
    }

    #[test]
    fn detects_synthetic_rotation() {
        for angle in [-4.0f32, -1.5, 2.0, 6.0] {
            let img = rotated_grid(angle);
            let detected = detect_angle(&img).expect("should find lines");
            assert!(
                (detected + angle).abs() < 0.5,
                "pattern at {angle}°: suggested {detected}° (want ~{}°)",
                -angle
            );
        }
    }

    #[test]
    fn level_image_needs_no_correction() {
        let img = rotated_grid(0.0);
        let detected = detect_angle(&img).expect("should find lines");
        assert!(detected.abs() < 0.3, "got {detected}");
    }

    #[test]
    fn featureless_image_gives_none() {
        let img = image::RgbaImage::from_pixel(400, 300, image::Rgba([128, 128, 128, 255]));
        assert_eq!(detect_angle(&img), None);
    }
}
