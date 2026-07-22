//! Tone curve math: monotone cubic interpolation for user point curves, the
//! parametric tone function (contrast/highlights/shadows/whites/blacks) and
//! composition of everything into per-channel LUTs.

use crate::engine::params::{EditParams, ToneCurves};

pub const LUT_SIZE: usize = 1024;

/// Monotone cubic (Fritsch–Carlson) through sorted control points, evaluated
/// on [0,1]. Guarantees no overshoot between points.
pub struct MonotoneCubic {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>, // tangents
}

impl MonotoneCubic {
    /// Points must be sorted by x with strictly increasing x (dedup close
    /// points first). Falls back to identity when fewer than 2 points.
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut pts: Vec<[f32; 2]> = points.to_vec();
        pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
        pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
        if pts.len() < 2 {
            pts = vec![[0.0, 0.0], [1.0, 1.0]];
        }
        let n = pts.len();
        let xs: Vec<f32> = pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p[1]).collect();

        // Secant slopes.
        let mut d = vec![0.0f32; n - 1];
        for i in 0..n - 1 {
            d[i] = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]).max(1e-6);
        }
        // Tangents.
        let mut ms = vec![0.0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                (d[i - 1] + d[i]) / 2.0
            };
        }
        // Fritsch–Carlson monotonicity fix.
        for i in 0..n - 1 {
            if d[i].abs() < 1e-9 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
            } else {
                let a = ms[i] / d[i];
                let b = ms[i + 1] / d[i];
                let s = a * a + b * b;
                if s > 9.0 {
                    let tau = 3.0 / s.sqrt();
                    ms[i] = tau * a * d[i];
                    ms[i + 1] = tau * b * d[i];
                }
            }
        }
        Self { xs, ys, ms }
    }

    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let i = match self
            .xs
            .binary_search_by(|v| v.total_cmp(&x))
        {
            Ok(i) => return self.ys[i],
            Err(i) => i - 1,
        };
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[i] + h10 * h * self.ms[i] + h01 * self.ys[i + 1] + h11 * h * self.ms[i + 1]
    }

    /// True when the curve is the two-point identity.
    pub fn is_identity(points: &[[f32; 2]]) -> bool {
        points.len() == 2
            && points[0] == [0.0, 0.0]
            && points[1] == [1.0, 1.0]
    }
}

/// The parametric tone mapping from the Basic panel, in display (gamma)
/// domain. All slider values are the raw -100..100 numbers.
pub fn parametric_tone(
    t: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
) -> f32 {
    let mut v = t.clamp(0.0, 1.0);

    // Contrast: smooth S around 0.5; kept monotone for |c1| <= 0.75.
    let c1 = contrast / 100.0 * 0.75;
    v += c1 * 2.0 * v * (1.0 - v) * (2.0 * v - 1.0);

    // Highlights: raise/recover the upper mids, pinning white.
    let h1 = highlights / 100.0;
    v += h1 * 0.35 * smoothstep(0.35, 0.95, v) * (1.0 - v);

    // Shadows: lift/crush the lower mids, pinning black.
    let s1 = shadows / 100.0;
    v += s1 * 0.35 * (1.0 - smoothstep(0.05, 0.65, v)) * v;

    // Whites: move the top end (white point).
    let w1 = whites / 100.0;
    v += w1 * 0.25 * smoothstep(0.25, 1.0, v);

    // Blacks: move the bottom end (black point).
    let b1 = blacks / 100.0;
    v += b1 * 0.25 * (1.0 - smoothstep(0.0, 0.75, v));

    v.clamp(0.0, 1.0)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Per-camera base render curve applied under everything for RAW files.
pub struct BaseCurve {
    pub baseline_ev: f32,
    pub curve: MonotoneCubic,
}

#[derive(serde::Deserialize)]
struct BaseCurveFile {
    baseline_ev: f32,
    points: Vec<[f32; 2]>,
}

impl BaseCurve {
    fn from_json(json: &str) -> Self {
        let f: BaseCurveFile =
            serde_json::from_str(json).expect("base curve asset parses");
        Self {
            baseline_ev: f.baseline_ev,
            curve: MonotoneCubic::new(&f.points),
        }
    }

    /// Select the base curve for a camera model string.
    pub fn for_camera(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        if m.contains("eos rp") {
            Self::from_json(include_str!(
                "../../assets/base_curves/canon_eos_rp.json"
            ))
        } else if m.contains("x100v") {
            Self::from_json(include_str!("../../assets/base_curves/fuji_x100v.json"))
        } else {
            Self::from_json(include_str!("../../assets/base_curves/generic.json"))
        }
    }
}

/// Composed per-channel LUTs, values in 0..1 over a 0..1 gamma-domain input.
pub struct ToneLut {
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
}

/// Build the tone LUT: base camera curve (RAW only) ∘ parametric sliders ∘
/// user composite curve ∘ user per-channel curves. Result is forced monotone.
pub fn build_lut(params: &EditParams, base: Option<&BaseCurve>) -> ToneLut {
    let curves = &params.tone_curve;
    let composite = curve_or_identity(&curves.composite);
    let cr = curve_or_identity(&curves.red);
    let cg = curve_or_identity(&curves.green);
    let cb = curve_or_identity(&curves.blue);

    let mut r = Vec::with_capacity(LUT_SIZE);
    let mut g = Vec::with_capacity(LUT_SIZE);
    let mut b = Vec::with_capacity(LUT_SIZE);
    for i in 0..LUT_SIZE {
        let t0 = i as f32 / (LUT_SIZE - 1) as f32;
        let t1 = match base {
            Some(bc) => bc.curve.eval(t0),
            None => t0,
        };
        let t2 = parametric_tone(
            t1,
            params.contrast,
            params.highlights,
            params.shadows,
            params.whites,
            params.blacks,
        );
        let t3 = match &composite {
            Some(c) => c.eval(t2),
            None => t2,
        };
        r.push(eval_opt(&cr, t3));
        g.push(eval_opt(&cg, t3));
        b.push(eval_opt(&cb, t3));
    }
    for lut in [&mut r, &mut g, &mut b] {
        enforce_monotone(lut);
    }
    ToneLut { r, g, b }
}

fn curve_or_identity(points: &[[f32; 2]]) -> Option<MonotoneCubic> {
    if MonotoneCubic::is_identity(points) {
        None
    } else {
        Some(MonotoneCubic::new(points))
    }
}

fn eval_opt(c: &Option<MonotoneCubic>, t: f32) -> f32 {
    match c {
        Some(c) => c.eval(t).clamp(0.0, 1.0),
        None => t,
    }
}

fn enforce_monotone(lut: &mut [f32]) {
    for i in 1..lut.len() {
        if lut[i] < lut[i - 1] {
            lut[i] = lut[i - 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_spline() {
        let c = MonotoneCubic::new(&[[0.0, 0.0], [1.0, 1.0]]);
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((c.eval(x) - x).abs() < 1e-4, "x={x} y={}", c.eval(x));
        }
    }

    #[test]
    fn no_overshoot() {
        // A steep step must not overshoot beyond its endpoints.
        let c = MonotoneCubic::new(&[[0.0, 0.0], [0.45, 0.05], [0.55, 0.95], [1.0, 1.0]]);
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let y = c.eval(x);
            assert!((-0.001..=1.001).contains(&y), "x={x} y={y}");
        }
        // And must be monotone.
        let mut prev = -1.0;
        for i in 0..=100 {
            let y = c.eval(i as f32 / 100.0);
            assert!(y >= prev - 1e-4, "not monotone at {i}: {y} < {prev}");
            prev = y;
        }
    }

    #[test]
    fn parametric_endpoints_stable() {
        // Contrast/highlights/shadows never move pure black or white.
        for c in [-100.0, 0.0, 100.0] {
            assert_eq!(parametric_tone(0.0, c, c, c, 0.0, 0.0), 0.0);
            assert_eq!(parametric_tone(1.0, c, c, c, 0.0, 0.0), 1.0);
        }
        // Positive contrast darkens below mid, brightens above.
        assert!(parametric_tone(0.25, 50.0, 0.0, 0.0, 0.0, 0.0) < 0.25);
        assert!(parametric_tone(0.75, 50.0, 0.0, 0.0, 0.0, 0.0) > 0.75);
        // Shadows slider lifts the shadows.
        assert!(parametric_tone(0.2, 0.0, 0.0, 60.0, 0.0, 0.0) > 0.2);
        // Highlights negative recovers (darkens) highlights.
        assert!(parametric_tone(0.8, 0.0, -60.0, 0.0, 0.0, 0.0) < 0.8);
    }

    #[test]
    fn lut_monotone_and_full_range() {
        let mut p = EditParams::default();
        p.contrast = 80.0;
        p.highlights = -70.0;
        p.shadows = 60.0;
        p.whites = 30.0;
        p.blacks = -40.0;
        p.tone_curve.composite = vec![[0.0, 0.05], [0.5, 0.6], [1.0, 0.95]];
        let lut = build_lut(&p, None);
        for ch in [&lut.r, &lut.g, &lut.b] {
            assert_eq!(ch.len(), LUT_SIZE);
            let mut prev = -1.0f32;
            for &v in ch.iter() {
                assert!((0.0..=1.0).contains(&v));
                assert!(v >= prev);
                prev = v;
            }
        }
    }

    #[test]
    fn base_curves_parse() {
        for model in ["Canon EOS RP", "X100V", "Unknown Cam"] {
            let bc = BaseCurve::for_camera(model);
            assert!(bc.curve.eval(0.5) > 0.0);
        }
    }
}
