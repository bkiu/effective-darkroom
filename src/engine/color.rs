//! Color math for white balance: CCT/tint → white point → Bradford chromatic
//! adaptation, expressed as a 3×3 matrix over linear sRGB.

pub type Mat3 = [[f32; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// sRGB (D65) to XYZ, row-major.
const RGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

const XYZ_TO_RGB: [[f64; 3]; 3] = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0556434, -0.2040259, 1.0572252],
];

/// Bradford cone response matrix.
const BRADFORD: [[f64; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

const BRADFORD_INV: [[f64; 3]; 3] = [
    [0.9869929, -0.1470543, 0.1599627],
    [0.4323053, 0.5183603, 0.0492912],
    [-0.0085287, 0.0400428, 0.9684867],
];

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn mat_vec(a: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        a[0][0] * v[0] + a[0][1] * v[1] + a[0][2] * v[2],
        a[1][0] * v[0] + a[1][1] * v[1] + a[1][2] * v[2],
        a[2][0] * v[0] + a[2][1] * v[1] + a[2][2] * v[2],
    ]
}

/// CCT (Kelvin) to CIE xy on the daylight locus (≥4000K, CIE D-series
/// polynomial) or the Planckian locus below (Kim et al. cubic spline approx).
pub fn cct_to_xy(cct: f32) -> (f64, f64) {
    let t = (cct as f64).clamp(1667.0, 25000.0);
    let x = if t >= 4000.0 {
        // CIE daylight locus.
        let r = 1000.0 / t;
        if t <= 7000.0 {
            0.244063 + 0.09911 * r + 2.9678 * r * r - 4.607 * r * r * r
        } else {
            0.23704 + 0.24748 * r + 1.9018 * r * r - 2.0064 * r * r * r
        }
    } else {
        // Planckian locus (Kim et al. 2002).
        let r = 1000.0 / t;
        -0.2661239 * r * r * r - 0.2343589 * r * r + 0.8776956 * r + 0.179910
    };
    let y = if t >= 4000.0 {
        -3.0 * x * x + 2.87 * x - 0.275
    } else if t >= 2222.0 {
        -0.9549476 * x * x * x - 1.37418593 * x * x + 2.09137015 * x - 0.16748867
    } else {
        -1.1063814 * x * x * x - 1.34811020 * x * x + 2.18555832 * x - 0.20219683
    };
    (x, y)
}

/// xy + Duv (CIE 1960 offset, +green/−magenta in v) → XYZ with Y=1.
fn white_xyz(x: f64, y: f64, duv: f64) -> [f64; 3] {
    // xy → CIE 1960 uv
    let denom = -2.0 * x + 12.0 * y + 3.0;
    let u = 4.0 * x / denom;
    let v = 6.0 * y / denom + duv;
    // uv → xy
    let denom2 = 2.0 * u - 8.0 * v + 4.0;
    let x2 = 3.0 * u / denom2;
    let y2 = 2.0 * v / denom2;
    [x2 / y2, 1.0, (1.0 - x2 - y2) / y2]
}

/// Lightroom-style tint (−150 green .. +150 magenta) to Duv. Positive tint
/// compensates *for* a green illuminant, i.e. the assumed white is greener.
fn tint_to_duv(tint: f32) -> f64 {
    tint as f64 / 3000.0
}

/// Bradford CAT taking colors balanced for `src_white` to `dst_white`.
fn bradford(src_white: [f64; 3], dst_white: [f64; 3]) -> [[f64; 3]; 3] {
    let s = mat_vec(&BRADFORD, src_white);
    let d = mat_vec(&BRADFORD, dst_white);
    let scale = [
        [d[0] / s[0], 0.0, 0.0],
        [0.0, d[1] / s[1], 0.0],
        [0.0, 0.0, d[2] / s[2]],
    ];
    mat_mul(&BRADFORD_INV, &mat_mul(&scale, &BRADFORD))
}

/// White balance matrix over linear sRGB for the temp/tint sliders.
///
/// `ref_temp`/`ref_tint` describe the white point the image is currently
/// balanced to (6500/0 for JPEGs; the As-Shot estimate for RAWs). Slider ==
/// reference yields the identity; raising temp warms the render.
pub fn wb_matrix(temp: f32, tint: f32, ref_temp: f32, ref_tint: f32) -> Mat3 {
    if (temp - ref_temp).abs() < 0.5 && (tint - ref_tint).abs() < 1e-3 {
        return IDENTITY;
    }
    let (sx, sy) = cct_to_xy(temp);
    let src = white_xyz(sx, sy, tint_to_duv(tint));
    let (dx, dy) = cct_to_xy(ref_temp);
    let dst = white_xyz(dx, dy, tint_to_duv(ref_tint));

    let cat = bradford(src, dst);
    let m = mat_mul(&XYZ_TO_RGB, &mat_mul(&cat, &RGB_TO_XYZ));
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = m[i][j] as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_identity() {
        assert_eq!(wb_matrix(6500.0, 0.0, 6500.0, 0.0), IDENTITY);
    }

    #[test]
    fn warmer_temp_boosts_red_over_blue() {
        let m = wb_matrix(10000.0, 0.0, 6500.0, 0.0);
        // Apply to a neutral gray: red should exceed blue (warmer render).
        let r = m[0][0] + m[0][1] + m[0][2];
        let b = m[2][0] + m[2][1] + m[2][2];
        assert!(r > 1.0 && b < 1.0, "r={r} b={b}");
    }

    #[test]
    fn positive_tint_pushes_magenta() {
        // Applying to neutral gray: green must drop relative to red and blue
        // (magenta compensation for an assumed greener illuminant).
        let m = wb_matrix(6500.0, 60.0, 6500.0, 0.0);
        let r = m[0][0] + m[0][1] + m[0][2];
        let g = m[1][0] + m[1][1] + m[1][2];
        let b = m[2][0] + m[2][1] + m[2][2];
        assert!(g < r && g < b, "r={r} g={g} b={b}");
    }

    #[test]
    fn cct_xy_sane() {
        let (x, y) = cct_to_xy(6500.0);
        assert!((x - 0.3135).abs() < 0.01, "{x}");
        assert!((y - 0.3236).abs() < 0.01, "{y}");
        let (x2, _) = cct_to_xy(2856.0);
        assert!(x2 > 0.44, "tungsten should be well right of daylight: {x2}");
    }
}
