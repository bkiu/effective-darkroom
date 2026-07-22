//! Minimal safe wrapper around the system LibRaw C API.
//!
//! Only the small surface Effective Darkroom needs is exposed: header-only
//! metadata (for import), embedded thumbnail extraction (for grid/loupe
//! previews) and a full linear 16-bit decode (for the develop engine).

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    improper_ctypes,
    unnecessary_transmutes
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum LibrawError {
    #[error("libraw: {0}")]
    Raw(String),
    #[error("path contains interior NUL byte")]
    BadPath,
    #[error("libraw_init returned null")]
    InitFailed,
    #[error("unsupported thumbnail format {0}")]
    UnsupportedThumb(i32),
}

pub type Result<T> = std::result::Result<T, LibrawError>;

fn check(code: i32) -> Result<()> {
    if code == 0 {
        return Ok(());
    }
    let msg = unsafe { CStr::from_ptr(sys::libraw_strerror(code)) };
    Err(LibrawError::Raw(format!(
        "{} (code {code})",
        msg.to_string_lossy()
    )))
}

/// Runtime LibRaw version string, e.g. "0.22.1-Release".
pub fn version() -> String {
    unsafe { CStr::from_ptr(sys::libraw_version()) }
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug, Clone)]
pub struct RawMetadata {
    pub make: String,
    pub model: String,
    /// Unix seconds (camera local time interpreted as such).
    pub timestamp: i64,
    /// Post-rotation output dimensions.
    pub width: u32,
    pub height: u32,
    pub iso: f32,
    pub shutter: f32,
    pub aperture: f32,
    pub focal_len: f32,
    /// LibRaw flip code (0 = none, 3 = 180, 5 = 270 CW, 6 = 90 CW).
    pub flip: i32,
}

/// An embedded preview image extracted from a RAW file.
pub enum Thumb {
    Jpeg(Vec<u8>),
    /// Interleaved 8-bit RGB.
    Rgb8 {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
}

/// A full linear decode: interleaved 16-bit RGB in linear sRGB primaries.
pub struct DecodedRaw {
    pub width: u32,
    pub height: u32,
    pub rgb16: Vec<u16>,
}

pub struct RawFile {
    data: *mut sys::libraw_data_t,
    unpacked: bool,
}

// A libraw_data_t is only touched through &mut self / owned self, and LibRaw
// instances are independent of each other, so moving one across threads is fine.
unsafe impl Send for RawFile {}

impl RawFile {
    /// Open and parse the header/metadata. Cheap; does not unpack pixel data.
    pub fn open(path: &Path) -> Result<Self> {
        let cpath =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| LibrawError::BadPath)?;
        let data = unsafe { sys::libraw_init(0) };
        if data.is_null() {
            return Err(LibrawError::InitFailed);
        }
        let rc = unsafe { sys::libraw_open_file(data, cpath.as_ptr()) };
        if rc != 0 {
            unsafe { sys::libraw_close(data) };
            return check(rc).map(|_| unreachable!());
        }
        Ok(Self {
            data,
            unpacked: false,
        })
    }

    pub fn metadata(&self) -> RawMetadata {
        let d = unsafe { &*self.data };
        let cstr = |bytes: &[i8]| -> String {
            let bytes: Vec<u8> = bytes
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&bytes).trim().to_string()
        };
        let flip = d.sizes.flip;
        // iwidth/iheight are pre-rotation; swap for 90/270 flips.
        let (mut w, mut h) = (d.sizes.iwidth as u32, d.sizes.iheight as u32);
        if flip == 5 || flip == 6 {
            std::mem::swap(&mut w, &mut h);
        }
        RawMetadata {
            make: cstr(&d.idata.make),
            model: cstr(&d.idata.model),
            timestamp: d.other.timestamp,
            width: w,
            height: h,
            iso: d.other.iso_speed,
            shutter: d.other.shutter,
            aperture: d.other.aperture,
            focal_len: d.other.focal_len,
            flip,
        }
    }

    /// White balance multipliers as shot (camera RGBG order).
    pub fn cam_mul(&self) -> [f32; 4] {
        unsafe { &*self.data }.color.cam_mul
    }

    /// Camera-RGB → XYZ-ish matrix from LibRaw (rows: R,G,B,G2; cols: XYZ).
    pub fn cam_xyz(&self) -> [[f32; 3]; 4] {
        unsafe { &*self.data }.color.cam_xyz
    }

    /// Extract the largest embedded preview (full-size JPEG on CR3/RAF).
    pub fn extract_thumb(&mut self) -> Result<Thumb> {
        check(unsafe { sys::libraw_unpack_thumb(self.data) })?;
        let mut errc: i32 = 0;
        let img = unsafe { sys::libraw_dcraw_make_mem_thumb(self.data, &mut errc) };
        if img.is_null() {
            return check(errc).map(|_| unreachable!());
        }
        let result = unsafe {
            let i = &*img;
            let data = std::slice::from_raw_parts(i.data.as_ptr(), i.data_size as usize);
            match i.type_ {
                sys::LibRaw_image_formats_LIBRAW_IMAGE_JPEG => Ok(Thumb::Jpeg(data.to_vec())),
                sys::LibRaw_image_formats_LIBRAW_IMAGE_BITMAP if i.bits == 8 && i.colors == 3 => {
                    Ok(Thumb::Rgb8 {
                        width: i.width as u32,
                        height: i.height as u32,
                        data: data.to_vec(),
                    })
                }
                t => Err(LibrawError::UnsupportedThumb(t as i32)),
            }
        };
        unsafe { sys::libraw_dcraw_clear_mem(img) };
        result
    }

    /// Full decode to linear 16-bit RGB in sRGB primaries, camera WB applied,
    /// no auto-brighten. `quality` is the demosaic: 3 = AHD (Bayer) /
    /// 3-pass Markesteijn (X-Trans).
    pub fn decode_linear(&mut self, quality: i32) -> Result<DecodedRaw> {
        {
            let d = unsafe { &mut *self.data };
            d.params.output_bps = 16;
            d.params.gamm[0] = 1.0; // linear transfer
            d.params.gamm[1] = 1.0;
            d.params.no_auto_bright = 1;
            d.params.use_camera_wb = 1;
            d.params.output_color = 1; // sRGB primaries
            d.params.user_qual = quality;
            d.params.user_flip = -1; // honor camera orientation
        }
        if !self.unpacked {
            check(unsafe { sys::libraw_unpack(self.data) })?;
            self.unpacked = true;
        }
        check(unsafe { sys::libraw_dcraw_process(self.data) })?;
        let mut errc: i32 = 0;
        let img = unsafe { sys::libraw_dcraw_make_mem_image(self.data, &mut errc) };
        if img.is_null() {
            return check(errc).map(|_| unreachable!());
        }
        let result = unsafe {
            let i = &*img;
            debug_assert_eq!(i.type_, sys::LibRaw_image_formats_LIBRAW_IMAGE_BITMAP);
            debug_assert_eq!(i.bits, 16);
            debug_assert_eq!(i.colors, 3);
            let px = std::slice::from_raw_parts(
                i.data.as_ptr() as *const u16,
                (i.data_size as usize) / 2,
            );
            DecodedRaw {
                width: i.width as u32,
                height: i.height as u32,
                rgb16: px.to_vec(),
            }
        };
        unsafe { sys::libraw_dcraw_clear_mem(img) };
        Ok(result)
    }
}

impl Drop for RawFile {
    fn drop(&mut self) {
        unsafe { sys::libraw_close(self.data) };
    }
}
