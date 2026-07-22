//! Non-destructive edit parameters. Serialized as JSON into the catalog's
//! `edits` table; `#[serde(default)]` on every field keeps old edits loadable
//! as the set grows.

use serde::{Deserialize, Serialize};

pub const PARAMS_VERSION: i64 = 1;

pub const TEMP_DEFAULT: f32 = 6500.0;
pub const TEMP_MIN: f32 = 2000.0;
pub const TEMP_MAX: f32 = 50000.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditParams {
    /// White balance temperature in Kelvin; 6500 = neutral for JPEGs,
    /// As-Shot value for RAWs.
    pub wb_temp: f32,
    /// Green (-150) .. magenta (+150).
    pub wb_tint: f32,

    /// EV, ±5.
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,

    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,

    pub vibrance: f32,
    pub saturation: f32,

    /// Chroma noise reduction 0-100 (Lightroom-style Color NR). Defaults to
    /// 25; fresh JPEG imports get 0 since cameras denoise their JPEGs.
    #[serde(default = "default_color_nr")]
    pub color_nr: f32,

    pub crop: CropParams,

    pub tone_curve: ToneCurves,
}

/// Straighten + crop. The crop rect is normalized (0..1) within the "safe
/// area" — the largest axis-aligned, source-aspect rectangle that fits
/// inside the rotated image — so it stays valid as the angle changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CropParams {
    /// Rotation in degrees, -45..45 (positive = clockwise).
    pub angle: f32,
    /// [x, y, w, h] in 0..1 of the safe area.
    pub rect: [f32; 4],
}

impl Default for CropParams {
    fn default() -> Self {
        Self {
            angle: 0.0,
            rect: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

impl CropParams {
    pub fn is_identity(&self) -> bool {
        self.angle == 0.0 && self.rect == [0.0, 0.0, 1.0, 1.0]
    }
}

fn default_color_nr() -> f32 {
    25.0
}

impl Default for EditParams {
    fn default() -> Self {
        Self {
            wb_temp: TEMP_DEFAULT,
            wb_tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            color_nr: default_color_nr(),
            crop: CropParams::default(),
            tone_curve: ToneCurves::default(),
        }
    }
}

/// Point curves for the composite and per-channel tone curve. Points are
/// (input, output) in 0..1, kept sorted by input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurves {
    pub composite: Vec<[f32; 2]>,
    pub red: Vec<[f32; 2]>,
    pub green: Vec<[f32; 2]>,
    pub blue: Vec<[f32; 2]>,
}

impl Default for ToneCurves {
    fn default() -> Self {
        let identity = vec![[0.0, 0.0], [1.0, 1.0]];
        Self {
            composite: identity.clone(),
            red: identity.clone(),
            green: identity.clone(),
            blue: identity,
        }
    }
}

impl EditParams {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("EditParams serializes")
    }

    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_and_forward_compat() {
        let mut p = EditParams::default();
        p.exposure = 1.25;
        p.wb_temp = 5200.0;
        let p2 = EditParams::from_json(&p.to_json());
        assert_eq!(p, p2);

        // Unknown/missing fields fall back to defaults.
        let p3 = EditParams::from_json(r#"{"exposure": 0.5, "future_field": 1}"#);
        assert_eq!(p3.exposure, 0.5);
        assert_eq!(p3.wb_temp, TEMP_DEFAULT);
    }
}
