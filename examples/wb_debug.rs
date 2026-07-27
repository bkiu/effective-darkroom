//! Print the As-Shot WB estimate and the WB matrix a given temp/tint slider
//! pair produces for a RAW file — for checking slider parity with Lightroom.
//!
//!   cargo run --example wb_debug -- file.CR3 [temp tint]

use effective_darkroom::engine::color;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let raw = libraw_ffi::RawFile::open(std::path::Path::new(path))?;
    let model = raw.metadata().model;
    let cam_mul = raw.cam_mul();
    let cam_xyz = raw.cam_xyz();
    println!("model: {model}");
    println!("cam_mul: {cam_mul:?}");
    println!("cam_xyz: {cam_xyz:?}");
    let as_shot = color::as_shot_wb(cam_mul, cam_xyz, &model);
    println!("as_shot estimate: {as_shot:?}");
    if args.len() >= 3 {
        let temp: f32 = args[1].parse()?;
        let tint: f32 = args[2].parse()?;
        let (rt, rtt) = as_shot.unwrap_or((6500.0, 0.0));
        let m = color::wb_matrix(temp, tint, rt, rtt);
        println!("wb_matrix({temp}, {tint}, ref {rt}, {rtt}):");
        for row in m {
            println!("  {row:?}");
        }
        let gray = [
            m[0][0] + m[0][1] + m[0][2],
            m[1][0] + m[1][1] + m[1][2],
            m[2][0] + m[2][1] + m[2][2],
        ];
        println!("gray -> {gray:?}  r/g={} b/g={}", gray[0] / gray[1], gray[2] / gray[1]);
    }
    Ok(())
}
