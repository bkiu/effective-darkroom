//! Color math for white balance, following the DNG SDK's temperature model
//! so temp/tint numbers are interchangeable with Lightroom's: Robertson CCT
//! over CIE 1960 uv with tint measured along the isotherm (dng_temperature),
//! and As-Shot estimation via Adobe's dual-illuminant ColorMatrix
//! interpolation (dng_color_spec::NeutralToXY). The slider white point is
//! applied as a Bradford chromatic adaptation over linear sRGB.

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

/// Robertson isotherm table from the DNG SDK (dng_temperature.cpp):
/// (mired, u, v, isotherm slope).
const ROBERTSON: [(f64, f64, f64, f64); 31] = [
    (0.0, 0.18006, 0.26352, -0.24341),
    (10.0, 0.18066, 0.26589, -0.25479),
    (20.0, 0.18133, 0.26846, -0.26876),
    (30.0, 0.18208, 0.27119, -0.28539),
    (40.0, 0.18293, 0.27407, -0.30470),
    (50.0, 0.18388, 0.27709, -0.32675),
    (60.0, 0.18494, 0.28021, -0.35156),
    (70.0, 0.18611, 0.28342, -0.37915),
    (80.0, 0.18740, 0.28668, -0.40955),
    (90.0, 0.18880, 0.28997, -0.44278),
    (100.0, 0.19032, 0.29326, -0.47888),
    (125.0, 0.19462, 0.30141, -0.58204),
    (150.0, 0.19962, 0.30921, -0.70471),
    (175.0, 0.20525, 0.31647, -0.84901),
    (200.0, 0.21142, 0.32312, -1.0182),
    (225.0, 0.21807, 0.32909, -1.2168),
    (250.0, 0.22511, 0.33439, -1.4512),
    (275.0, 0.23247, 0.33904, -1.7298),
    (300.0, 0.24010, 0.34308, -2.0637),
    (325.0, 0.24702, 0.34655, -2.4681),
    (350.0, 0.25591, 0.34951, -2.9641),
    (375.0, 0.26400, 0.35200, -3.5814),
    (400.0, 0.27218, 0.35407, -4.3633),
    (425.0, 0.28039, 0.35577, -5.3762),
    (450.0, 0.28863, 0.35714, -6.7262),
    (475.0, 0.29685, 0.35823, -8.5955),
    (500.0, 0.30505, 0.35907, -11.324),
    (525.0, 0.31320, 0.35968, -15.628),
    (550.0, 0.32129, 0.36011, -23.325),
    (575.0, 0.32931, 0.36038, -40.770),
    (600.0, 0.33724, 0.36051, -116.45),
];

/// Adobe's tint scale: uv distance along the isotherm × −3000. Positive tint
/// is a greener assumed illuminant, i.e. magenta compensation in the render.
const TINT_SCALE: f64 = -3000.0;

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

/// (temp, tint) → CIE 1960 uv. Port of dng_temperature::Get_xy_coord.
pub fn temp_tint_to_uv(temp: f32, tint: f32) -> (f64, f64) {
    let r = 1.0e6 / (temp as f64).clamp(TEMP_MIN_F64, TEMP_MAX_F64);
    let offset = tint as f64 / TINT_SCALE;
    for index in 0..=29 {
        if r < ROBERTSON[index + 1].0 || index == 29 {
            let f = ((ROBERTSON[index + 1].0 - r)
                / (ROBERTSON[index + 1].0 - ROBERTSON[index].0))
                .clamp(0.0, 1.0);
            let u = ROBERTSON[index].1 * f + ROBERTSON[index + 1].1 * (1.0 - f);
            let v = ROBERTSON[index].2 * f + ROBERTSON[index + 1].2 * (1.0 - f);
            // Unit vectors along each line's isotherm slope, blended.
            let (uu1, vv1) = (1.0, ROBERTSON[index].3);
            let (uu2, vv2) = (1.0, ROBERTSON[index + 1].3);
            let len1 = (1.0 + vv1 * vv1).sqrt();
            let len2 = (1.0 + vv2 * vv2).sqrt();
            let uu = uu1 / len1 * f + uu2 / len2 * (1.0 - f);
            let vv = vv1 / len1 * f + vv2 / len2 * (1.0 - f);
            let len = (uu * uu + vv * vv).sqrt();
            return (u + uu / len * offset, v + vv / len * offset);
        }
    }
    unreachable!()
}

/// CIE 1960 uv → (temp, tint). Port of dng_temperature::Set_xy_coord.
pub fn uv_to_temp_tint(u: f64, v: f64) -> (f64, f64) {
    let mut last_dt = 0.0;
    let mut last_du = 0.0;
    let mut last_dv = 0.0;
    for index in 1..=30 {
        let len = (1.0 + ROBERTSON[index].3 * ROBERTSON[index].3).sqrt();
        let mut du = 1.0 / len;
        let mut dv = ROBERTSON[index].3 / len;
        let mut uu = u - ROBERTSON[index].1;
        let mut vv = v - ROBERTSON[index].2;
        let dt = -uu * dv + vv * du;
        if dt <= 0.0 || index == 30 {
            let dt = -dt.min(0.0);
            let f = if index == 1 { 0.0 } else { dt / (last_dt + dt) };
            let temp = 1.0e6 / (ROBERTSON[index - 1].0 * f + ROBERTSON[index].0 * (1.0 - f));
            uu = u - (ROBERTSON[index - 1].1 * f + ROBERTSON[index].1 * (1.0 - f));
            vv = v - (ROBERTSON[index - 1].2 * f + ROBERTSON[index].2 * (1.0 - f));
            du = du * (1.0 - f) + last_du * f;
            dv = dv * (1.0 - f) + last_dv * f;
            let len = (du * du + dv * dv).sqrt();
            let tint = (uu * du / len + vv * dv / len) * TINT_SCALE;
            return (temp, tint);
        }
        last_dt = dt;
        last_du = du;
        last_dv = dv;
    }
    unreachable!()
}

/// xy → CIE 1960 uv.
fn xy_to_uv(x: f64, y: f64) -> (f64, f64) {
    let denom = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / denom, 6.0 * y / denom)
}

/// CIE 1960 uv → XYZ with Y=1.
fn uv_to_white_xyz(u: f64, v: f64) -> [f64; 3] {
    let denom = 2.0 * u - 8.0 * v + 4.0;
    let x = 3.0 * u / denom;
    let y = 2.0 * v / denom;
    [x / y, 1.0, (1.0 - x - y) / y]
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
    let (su, sv) = temp_tint_to_uv(temp, tint);
    let (du, dv) = temp_tint_to_uv(ref_temp, ref_tint);
    let src = uv_to_white_xyz(su, sv);
    let dst = uv_to_white_xyz(du, dv);

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

/// Adobe ColorMatrix1 (Standard Illuminant A, ~2850K) for the supported
/// cameras, extracted from Adobe DNG Converter output. LibRaw's cam_xyz is
/// Adobe's ColorMatrix2 (D65), so together they give Lightroom's exact
/// dual-illuminant camera→XYZ model.
fn color_matrix_1(model: &str) -> Option<[[f64; 3]; 3]> {
    let model = model.trim();
    if model.contains("EOS RP") {
        Some([
            [0.9665, -0.4042, -0.0053],
            [-0.3990, 1.1404, 0.2962],
            [-0.0220, 0.0939, 0.6291],
        ])
    } else if model == "X100V" {
        Some([
            [1.7835, -1.1702, 0.1616],
            [-0.2581, 1.0513, 0.2395],
            [0.0196, 0.0320, 0.6444],
        ])
    } else {
        None
    }
}

/// Std-A / D65 calibration illuminant temperatures (DNG spec).
const ILLUM1_TEMP: f64 = 2850.0;
const ILLUM2_TEMP: f64 = 6500.0;

/// Estimate the As-Shot (temp, tint) from LibRaw's camera WB multipliers and
/// camera→XYZ matrix, matching Lightroom's readout: the camera neutral is
/// mapped through the ColorMatrix interpolated between the Std-A and D65
/// calibrations by inverse CCT, iterated to self-consistency
/// (dng_color_spec::NeutralToXY). Returns None on degenerate metadata.
pub fn as_shot_wb(cam_mul: [f32; 4], cam_xyz: [[f32; 3]; 4], model: &str) -> Option<(f32, f32)> {
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
    let cm2 = [
        [cam_xyz[0][0] as f64, cam_xyz[0][1] as f64, cam_xyz[0][2] as f64],
        [cam_xyz[1][0] as f64, cam_xyz[1][1] as f64, cam_xyz[1][2] as f64],
        [cam_xyz[2][0] as f64, cam_xyz[2][1] as f64, cam_xyz[2][2] as f64],
    ];
    let cm1 = color_matrix_1(model).unwrap_or(cm2);

    let mut temp = 5000.0f64;
    let mut tint = 0.0f64;
    for _ in 0..30 {
        let w = ((1.0 / temp - 1.0 / ILLUM2_TEMP) / (1.0 / ILLUM1_TEMP - 1.0 / ILLUM2_TEMP))
            .clamp(0.0, 1.0);
        let mut m = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = cm1[i][j] * w + cm2[i][j] * (1.0 - w);
            }
        }
        let inv = invert3(&m)?;
        let xyz = mat_vec(&inv, cam_neutral);
        let sum = xyz[0] + xyz[1] + xyz[2];
        if !(sum.is_finite() && sum > 0.0) {
            return None;
        }
        let (u, v) = xy_to_uv(xyz[0] / sum, xyz[1] / sum);
        let (t, tt) = uv_to_temp_tint(u, v);
        let done = (t - temp).abs() < 0.1;
        temp = t;
        tint = tt;
        if done {
            break;
        }
    }
    if !(1500.0..=60000.0).contains(&temp) {
        return None;
    }
    Some((
        temp.clamp(TEMP_MIN_F64, TEMP_MAX_F64) as f32,
        tint.clamp(-150.0, 150.0) as f32,
    ))
}

const TEMP_MIN_F64: f64 = 2000.0;
const TEMP_MAX_F64: f64 = 50000.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// EOS RP cam_xyz (Adobe ColorMatrix2, scaled by 1e-4 from LibRaw).
    const EOS_RP_CAM_XYZ: [[f32; 3]; 4] = [
        [0.8608, -0.2097, -0.1178],
        [-0.5425, 1.3265, 0.2383],
        [-0.1149, 0.2238, 0.568],
        [0.0; 3],
    ];

    #[test]
    fn as_shot_from_eos_rp_tungsten() {
        // Values captured from IMG_4186.CR3 (indoor candlelight/tungsten):
        // cam_mul strongly favors blue multiplication => low CCT estimate.
        let cam_mul = [1208.0, 1024.0, 3246.0, 1024.0];
        let (temp, tint) = as_shot_wb(cam_mul, EOS_RP_CAM_XYZ, "EOS RP").unwrap();
        assert!(
            (2000.0..4500.0).contains(&temp),
            "expected warm illuminant, got {temp}K tint {tint}"
        );
    }

    #[test]
    fn as_shot_matches_lightroom_readout() {
        // IMG_4684.CR3: Lightroom reads this frame's As Shot near 3350/+34;
        // the dual-matrix interpolation must land in the same neighborhood
        // (the old D65-only estimate said 3460/+38).
        let cam_mul = [1517.0, 1024.0, 3039.0, 1024.0];
        let (temp, tint) = as_shot_wb(cam_mul, EOS_RP_CAM_XYZ, "EOS RP").unwrap();
        assert!((3250.0..3450.0).contains(&temp), "temp {temp}");
        assert!((28.0..40.0).contains(&tint), "tint {tint}");
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
    fn temp_tint_uv_roundtrip() {
        for (temp, tint) in [(3659.0, 42.0), (2400.0, -30.0), (5500.0, 10.0), (12000.0, 0.0)] {
            let (u, v) = temp_tint_to_uv(temp, tint);
            let (t2, tt2) = uv_to_temp_tint(u, v);
            assert!((t2 - temp as f64).abs() < temp as f64 * 0.002, "{temp} -> {t2}");
            assert!((tt2 - tint as f64).abs() < 0.1, "{tint} -> {tt2}");
        }
    }

    #[test]
    fn d65_reads_daylight_tint() {
        // D65 sits slightly green of the Planckian locus; in Adobe's model it
        // reads ≈6500K/+10 (Lightroom's Daylight preset is 5500/+10).
        let (u, v) = xy_to_uv(0.3127, 0.3290);
        let (temp, tint) = uv_to_temp_tint(u, v);
        assert!((6400.0..6650.0).contains(&temp), "{temp}");
        assert!((8.0..12.0).contains(&tint), "{tint}");
    }
}
