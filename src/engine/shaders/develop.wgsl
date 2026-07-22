// Develop pipeline: fullscreen pass over the linear source texture.
//
// Source is sampled as linear light (Rgba8UnormSrgb for JPEG sources,
// Rgba16Float for RAW). Linear-domain ops run first (white balance,
// exposure), then the result is sRGB-encoded and color ops that want a
// perceptual domain (saturation, vibrance) run on the encoded values.

struct Uniforms {
    wb0: vec4<f32>,          // white balance matrix, rows
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    exposure_gain: f32,      // 2^EV
    saturation: f32,         // -1..1
    vibrance: f32,           // -1..1
    _pad0: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle.
    var out: VsOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var rgb = textureSampleLevel(src_tex, src_samp, in.uv, 0.0).rgb;

    // --- linear domain ---
    rgb = vec3<f32>(dot(u.wb0.xyz, rgb), dot(u.wb1.xyz, rgb), dot(u.wb2.xyz, rgb));
    rgb = rgb * u.exposure_gain;
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));

    // --- encoded (perceptual-ish) domain ---
    var enc = srgb_encode(rgb);

    let luma = dot(enc, vec3<f32>(0.2126, 0.7152, 0.0722));
    let mx = max(enc.r, max(enc.g, enc.b));
    let mn = min(enc.r, min(enc.g, enc.b));
    let sat_est = (mx - mn) / max(mx, 1e-4);

    // Vibrance: boost mostly the low-saturation pixels.
    let vib_k = 1.0 + u.vibrance * (1.0 - sat_est);
    enc = mix(vec3<f32>(luma), enc, vib_k);
    // Saturation: uniform.
    enc = mix(vec3<f32>(luma), enc, 1.0 + u.saturation);
    enc = clamp(enc, vec3<f32>(0.0), vec3<f32>(1.0));

    return vec4<f32>(enc, 1.0);
}
