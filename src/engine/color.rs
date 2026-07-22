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

fn invert3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    let mut inv = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let (a, b, c, d) = (
                m[(j + 1) % 3][(i + 1) % 3],
                m[(j + 1) % 3][(i + 2) % 3],
                m[(j + 2) % 3][(i + 1) % 3],
                m[(j + 2) % 3][(i + 2) % 3],
            );
            inv[i][j] = (a * d - b * c) * inv_det;
        }
    }
    Some(inv)
}

/// xy → CIE 1960 uv.
fn xy_to_uv(x: f64, y: f64) -> (f64, f64) {
    let denom = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / denom, 6.0 * y / denom)
}

/// Estimate the As-Shot (temp, tint) from LibRaw's camera WB multipliers and
/// camera→XYZ matrix. Returns None if the metadata is degenerate.
pub fn as_shot_wb(cam_mul: [f32; 4], cam_xyz: [[f32; 3]; 4]) -> Option<(f32, f32)> {
    if cam_mul[0] <= 0.0 || cam_mul[1] <= 0.0 || cam_mul[2] <= 0.0 {
        return None;
    }
    // A neutral patch under the shot illuminant has camera RGB ∝ 1/cam_mul.
    let cam_neutral = [
        1.0 / cam_mul[0] as f64,
        1.0 / cam_mul[1] as f64,
        1.0 / cam_mul[2] as f64,
    ];
    // cam_xyz maps XYZ → camera RGB (rows R,G,B).
    let m = [
        [cam_xyz[0][0] as f64, cam_xyz[0][1] as f64, cam_xyz[0][2] as f64],
        [cam_xyz[1][0] as f64, cam_xyz[1][1] as f64, cam_xyz[1][2] as f64],
        [cam_xyz[2][0] as f64, cam_xyz[2][1] as f64, cam_xyz[2][2] as f64],
    ];
    let inv = invert3(&m)?;
    let xyz = mat_vec(&inv, cam_neutral);
    let sum = xyz[0] + xyz[1] + xyz[2];
    if !(sum.is_finite() && sum > 0.0) {
        return None;
    }
    let (x, y) = (xyz[0] / sum, xyz[1] / sum);

    // McCamy's CCT approximation.
    let n = (x - 0.3320) / (0.1858 - y);
    let cct = 449.0 * n * n * n + 3525.0 * n * n + 6823.3 * n + 5520.33;
    if !(1500.0..=60000.0).contains(&cct) {
        return None;
    }
    let cct = cct.clamp(TEMP_MIN_F64, TEMP_MAX_F64);

    // Duv relative to the same locus wb_matrix uses, so slider == estimate
    // gives the identity.
    let (lx, ly) = cct_to_xy(cct as f32);
    let (_, lv) = xy_to_uv(lx, ly);
    let (_, v) = xy_to_uv(x, y);
    let tint = ((v - lv) * 3000.0).clamp(-150.0, 150.0);
    Some((cct as f32, tint as f32))
}

const TEMP_MIN_F64: f64 = 2000.0;
const TEMP_MAX_F64: f64 = 50000.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_shot_from_eos_rp_tungsten() {
        // Values captured from IMG_4186.CR3 (indoor candlelight/tungsten):
        // cam_mul strongly favors blue multiplication => low CCT estimate.
        let cam_mul = [1208.0, 1024.0, 3246.0, 1024.0];
        // EOS RP cam_xyz (from LibRaw's Colordata, scaled by 1e-4).
        let cam_xyz = [
            [0.8608, -0.2097, -0.1178],
            [-0.5425, 1.3265, 0.2383],
            [-0.1149, 0.1781, 0.5619],
        ];
        let cam_xyz4 = [cam_xyz[0], cam_xyz[1], cam_xyz[2], [0.0; 3]];
        let (temp, tint) = as_shot_wb(cam_mul, cam_xyz4).unwrap();
        assert!(
            (2000.0..4500.0).contains(&temp),
            "expected warm illuminant, got {temp}K tint {tint}"
        );
    }

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
