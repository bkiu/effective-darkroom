// Prepasses for texture/clarity and chroma noise reduction: adjusted YCbCr
// extraction, separable gaussian blur, and plain downsample.

struct DevelopUniforms {
    wb0: vec4<f32>,
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    exposure_gain: f32,
    saturation: f32,
    vibrance: f32,
    texture_amt: f32,
    clarity_amt: f32,
    dehaze_amt: f32,
    use_blur: f32,
    color_nr: f32,
};

struct BlurUniforms {
    // Texel step in UV for one source pixel, oriented along the blur axis.
    dir: vec2<f32>,
    sigma: f32,
    _pad0: f32,
};

@group(0) @binding(0) var in_tex: texture_2d<f32>;
@group(0) @binding(1) var in_samp: sampler;
@group(0) @binding(2) var<uniform> dev: DevelopUniforms;
@group(0) @binding(3) var<uniform> blur: BlurUniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn srgb_encode1(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Adjusted, encoded YCbCr of the source image (must mirror develop.wgsl's
// linear-domain steps so downstream deltas are computed in the same domain).
// r = Y (used by the texture/clarity USM), gb = Cb/Cr (used by chroma NR).
@fragment
fn fs_ycc(in: VsOut) -> @location(0) vec4<f32> {
    var rgb = textureSampleLevel(in_tex, in_samp, in.uv, 0.0).rgb;
    rgb = vec3<f32>(dot(dev.wb0.xyz, rgb), dot(dev.wb1.xyz, rgb), dot(dev.wb2.xyz, rgb));
    rgb = clamp(rgb * dev.exposure_gain, vec3<f32>(0.0), vec3<f32>(1.0));
    let enc = srgb_encode1(rgb);
    let y = dot(enc, vec3<f32>(0.2126, 0.7152, 0.0722));
    let cb = (enc.b - y) / 1.8556;
    let cr = (enc.r - y) / 1.5748;
    return vec4<f32>(y, cb, cr, 1.0);
}

// 1D gaussian along blur.dir over all channels; sigma < 0.5 degenerates to a
// passthrough (used for the plain downsample stage).
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    if (blur.sigma < 0.5) {
        return vec4<f32>(textureSampleLevel(in_tex, in_samp, in.uv, 0.0).rgb, 1.0);
    }
    let radius = i32(ceil(blur.sigma * 3.0));
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = -radius; i <= radius; i = i + 1) {
        let w = exp(-0.5 * f32(i * i) / (blur.sigma * blur.sigma));
        sum = sum + w * textureSampleLevel(in_tex, in_samp, in.uv + f32(i) * blur.dir, 0.0).rgb;
        wsum = wsum + w;
    }
    return vec4<f32>(sum / wsum, 1.0);
}
