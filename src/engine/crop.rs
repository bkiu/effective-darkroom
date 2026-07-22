//! Geometry for straighten + crop: safe-area computation and the affine
//! transform that maps output UVs to source UVs.

use crate::engine::params::CropParams;

/// The largest axis-aligned rectangle with the source aspect ratio that fits
/// fully inside the source image rotated by `angle` degrees. Returned in
/// source-pixel units, centered on the image center.
pub fn safe_area(w: f32, h: f32, angle: f32) -> (f32, f32) {
    let a = angle.to_radians().abs();
    let (s, c) = (a.sin(), a.cos());
    // Scale k so that (k*w, k*h) fits inside the rotated w×h rectangle.
    let k1 = w / (w * c + h * s);
    let k2 = h / (w * s + h * c);
    let k = k1.min(k2).min(1.0);
    (w * k, h * k)
}

/// Bounding box of the source rotated by `angle` degrees.
pub fn bounding_box(w: f32, h: f32, angle: f32) -> (f32, f32) {
    let a = angle.to_radians().abs();
    let (s, c) = (a.sin(), a.cos());
    (w * c + h * s, w * s + h * c)
}

/// Output pixel size for a render. `full_preview` renders the whole rotated
/// bounding box (crop tool); otherwise the crop rect.
pub fn output_size(crop: &CropParams, w: u32, h: u32, full_preview: bool) -> (u32, u32) {
    let (wf, hf) = (w as f32, h as f32);
    let (ow, oh) = if full_preview {
        bounding_box(wf, hf, crop.angle)
    } else {
        let (mw, mh) = safe_area(wf, hf, crop.angle);
        (mw * crop.rect[2], mh * crop.rect[3])
    };
    ((ow.round() as u32).max(1), (oh.round() as u32).max(1))
}

/// Affine rows mapping output UV → source UV:
/// `src_uv.x = r0[0]*u + r0[1]*v + r0[2]`, same for y with r1.
pub fn uv_affine(
    crop: &CropParams,
    w: u32,
    h: u32,
    full_preview: bool,
) -> ([f32; 3], [f32; 3]) {
    let (wf, hf) = (w as f32, h as f32);
    let rad = crop.angle.to_radians();
    let (s, c) = (rad.sin(), rad.cos());

    // Output rect in the rotated frame, in source-pixel units, relative to
    // the image center.
    let (rw, rh, rx, ry) = if full_preview {
        let (bw, bh) = bounding_box(wf, hf, crop.angle);
        (bw, bh, -bw / 2.0, -bh / 2.0)
    } else {
        let (mw, mh) = safe_area(wf, hf, crop.angle);
        let cw = mw * crop.rect[2];
        let ch = mh * crop.rect[3];
        let cx = -mw / 2.0 + crop.rect[0] * mw;
        let cy = -mh / 2.0 + crop.rect[1] * mh;
        (cw, ch, cx, cy)
    };

    // P = (rx + u*rw, ry + v*rh) in the rotated frame (center origin).
    // Source px = center + Rot(-angle) * P; positive angle rotates the image
    // clockwise, so sampling uses the inverse rotation.
    // Rot(-a) = [c, s; -s, c] for a = rad.
    // src.x = w/2 + c*(rx + u*rw) + s*(ry + v*rh)
    // src.y = h/2 - s*(rx + u*rw) + c*(ry + v*rh)
    let r0 = [
        c * rw / wf,
        s * rh / wf,
        (wf / 2.0 + c * rx + s * ry) / wf,
    ];
    let r1 = [
        -s * rw / hf,
        c * rh / hf,
        (hf / 2.0 - s * rx + c * ry) / hf,
    ];
    (r0, r1)
}

/// Where the safe area lands within the full-preview output, as a normalized
/// [x, y, w, h] rect (for drawing the crop overlay).
pub fn safe_area_in_preview(crop: &CropParams, w: u32, h: u32) -> [f32; 4] {
    let (wf, hf) = (w as f32, h as f32);
    let (bw, bh) = bounding_box(wf, hf, crop.angle);
    let (mw, mh) = safe_area(wf, hf, crop.angle);
    [
        (bw - mw) / 2.0 / bw,
        (bh - mh) / 2.0 / bh,
        mw / bw,
        mh / bh,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(angle: f32, rect: [f32; 4]) -> CropParams {
        CropParams { angle, rect }
    }

    #[test]
    fn identity_is_full_frame() {
        let p = CropParams::default();
        assert_eq!(output_size(&p, 6000, 4000, false), (6000, 4000));
        let (r0, r1) = uv_affine(&p, 6000, 4000, false);
        // uv (0,0) → (0,0); uv (1,1) → (1,1)
        assert!((r0[2]).abs() < 1e-5 && (r1[2]).abs() < 1e-5);
        assert!((r0[0] + r0[2] - 1.0).abs() < 1e-5);
        assert!((r1[1] + r1[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn safe_area_shrinks_with_angle() {
        let (mw, mh) = safe_area(6000.0, 4000.0, 5.0);
        assert!(mw < 6000.0 && mh < 4000.0);
        // Aspect preserved.
        assert!((mw / mh - 1.5).abs() < 1e-3);
    }

    #[test]
    fn rotated_corners_stay_inside_source() {
        // Every corner of the safe-area crop, mapped through the affine,
        // must stay inside the source UV space.
        for angle in [-10.0f32, -3.0, 2.0, 7.5, 20.0] {
            let p = params(angle, [0.0, 0.0, 1.0, 1.0]);
            let (r0, r1) = uv_affine(&p, 6264, 4180, false);
            for (u, v) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let x = r0[0] * u + r0[1] * v + r0[2];
                let y = r1[0] * u + r1[1] * v + r1[2];
                assert!(
                    (-1e-4..=1.0001).contains(&x) && (-1e-4..=1.0001).contains(&y),
                    "angle {angle}: uv ({u},{v}) mapped outside: ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn sub_crop_maps_within_safe_area() {
        let p = params(4.0, [0.25, 0.1, 0.5, 0.6]);
        let (r0, r1) = uv_affine(&p, 6000, 4000, false);
        for (u, v) in [(0.0f32, 0.0f32), (1.0, 1.0), (0.5, 0.5)] {
            let x = r0[0] * u + r0[1] * v + r0[2];
            let y = r1[0] * u + r1[1] * v + r1[2];
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
        }
    }

    #[test]
    fn preview_contains_out_of_source_regions() {
        // The bounding-box preview must map some corner outside the source
        // (that's the dark triangle the crop tool shows).
        let p = params(5.0, [0.0, 0.0, 1.0, 1.0]);
        let (r0, r1) = uv_affine(&p, 6000, 4000, true);
        let x = r0[2]; // uv (0,0)
        let y = r1[2];
        assert!(!(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y));
    }
}
