// Develop pipeline: fullscreen pass over the linear source texture.
//
// Source is sampled as linear light (Rgba8UnormSrgb for JPEG sources,
// Rgba16Float for RAW). Linear-domain ops run first (white balance,
// exposure, dehaze), then the pixel is sRGB-encoded; local contrast
// (texture/clarity, from blurred-luma prepasses) and the tone LUT operate in
// the encoded domain, followed by vibrance/saturation.

struct Uniforms {
    wb0: vec4<f32>,          // white balance matrix, rows
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    exposure_gain: f32,      // 2^EV (includes RAW baseline exposure)
    saturation: f32,         // -1..1
    vibrance: f32,           // -1..1
    texture_amt: f32,        // -1..1
    clarity_amt: f32,        // -1..1
    dehaze_amt: f32,         // -1..1
    use_blur: f32,           // 1.0 when blur textures are valid
    color_nr: f32,           // 0..1 chroma noise reduction strength
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> u: Uniforms;
@group(0) @binding(3) var lut_tex: texture_2d<f32>;
@group(0) @binding(4) var blur_small_tex: texture_2d<f32>;
@group(0) @binding(5) var blur_large_tex: texture_2d<f32>;

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

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn luma_of(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn lut_sample(v: f32) -> vec2<f32> {
    // 1024-wide LUT: map 0..1 to texel centers.
    return vec2<f32>(v * (1023.0 / 1024.0) + 0.5 / 1024.0, 0.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var rgb = textureSampleLevel(src_tex, src_samp, in.uv, 0.0).rgb;

    // --- linear domain ---
    rgb = vec3<f32>(dot(u.wb0.xyz, rgb), dot(u.wb1.xyz, rgb), dot(u.wb2.xyz, rgb));
    rgb = rgb * u.exposure_gain;
    rgb = max(rgb, vec3<f32>(0.0));

    // Dehaze: per-pixel min-channel veil approximation with white airlight.
    if (abs(u.dehaze_amt) > 0.001) {
        if (u.dehaze_amt > 0.0) {
            let m = min(rgb.r, min(rgb.g, rgb.b));
            let veil = u.dehaze_amt * 0.7 * min(m, 1.0);
            let t = max(1.0 - veil, 0.15);
            rgb = (rgb - vec3<f32>(veil)) / t;
            rgb = max(rgb, vec3<f32>(0.0));
        } else {
            rgb = mix(rgb, vec3<f32>(0.8), -u.dehaze_amt * 0.3);
        }
    }
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));

    // --- encoded (display) domain ---
    var enc = srgb_encode(rgb);

    // Chroma noise reduction: replace high-frequency chroma with the blurred
    // half-res chroma, keeping luminance untouched.
    if (u.use_blur > 0.5 && u.color_nr > 0.001) {
        let y = luma_of(enc);
        let cb = (enc.b - y) / 1.8556;
        let cr = (enc.r - y) / 1.5748;
        let blurred = textureSampleLevel(blur_small_tex, src_samp, in.uv, 0.0);
        let f = 1.0 - exp(-u.color_nr * 5.5);
        let cb2 = mix(cb, blurred.g, f);
        let cr2 = mix(cr, blurred.b, f);
        let r = y + 1.5748 * cr2;
        let b = y + 1.8556 * cb2;
        let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
        enc = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // Texture/Clarity: unsharp-mask local contrast on luminance.
    if (u.use_blur > 0.5 && (abs(u.texture_amt) > 0.001 || abs(u.clarity_amt) > 0.001)) {
        let l = luma_of(enc);
        let bs = textureSampleLevel(blur_small_tex, src_samp, in.uv, 0.0).r;
        let bl = textureSampleLevel(blur_large_tex, src_samp, in.uv, 0.0).r;
        var delta = u.texture_amt * 0.6 * (l - bs) + u.clarity_amt * 0.75 * (l - bl);
        delta = clamp(delta, -0.3, 0.3);
        let scale = (clamp(l + delta, 0.0, 1.0)) / max(l, 1e-4);
        enc = clamp(enc * scale, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // Tone LUT (base curve + parametric sliders + user curves).
    enc = vec3<f32>(
        textureSampleLevel(lut_tex, src_samp, lut_sample(enc.r), 0.0).r,
        textureSampleLevel(lut_tex, src_samp, lut_sample(enc.g), 0.0).g,
        textureSampleLevel(lut_tex, src_samp, lut_sample(enc.b), 0.0).b,
    );

    let luma = luma_of(enc);
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
