//! Smoke test: print metadata (and optionally extract the embedded thumb)
//! for each RAW file passed on the command line.
//!
//!   cargo run -p libraw-ffi --example probe -- file.CR3 [--thumb] [--decode]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let thumb = args.iter().any(|a| a == "--thumb");
    let decode = args.iter().any(|a| a == "--decode");
    println!("LibRaw {}", libraw_ffi::version());

    for path in args.iter().filter(|a| !a.starts_with("--")) {
        let mut raw = libraw_ffi::RawFile::open(std::path::Path::new(path))?;
        let md = raw.metadata();
        println!(
            "{path}: {} {} | {}x{} flip={} | ts={} | ISO {} f/{} {}s {}mm",
            md.make,
            md.model,
            md.width,
            md.height,
            md.flip,
            md.timestamp,
            md.iso,
            md.aperture,
            md.shutter,
            md.focal_len
        );
        println!("  cam_mul: {:?}", raw.cam_mul());
        if thumb {
            let t = std::time::Instant::now();
            match raw.extract_thumb()? {
                libraw_ffi::Thumb::Jpeg(bytes) => {
                    println!("  thumb: JPEG {} bytes in {:?}", bytes.len(), t.elapsed())
                }
                libraw_ffi::Thumb::Rgb8 { width, height, .. } => {
                    println!("  thumb: RGB8 {width}x{height} in {:?}", t.elapsed())
                }
            }
        }
        if decode {
            let t = std::time::Instant::now();
            let img = raw.decode_linear(3)?;
            let mx = img.rgb16.iter().copied().max().unwrap_or(0);
            println!(
                "  decode: {}x{} linear16, max={} in {:?}",
                img.width,
                img.height,
                mx,
                t.elapsed()
            );
        }
    }
    Ok(())
}
