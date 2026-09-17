//! Exif for exported JPEGs.
//!
//! The develop pipeline hands back bare pixels, so a freshly encoded JPEG has
//! no metadata at all. This rebuilds an Exif APP1 segment for it:
//!
//! - JPEG sources keep their own Exif verbatim, so lens, GPS, exposure mode
//!   and anything else the camera or phone wrote survives the round trip.
//! - RAW sources get one reconstructed from rawler's parse of the file, which
//!   knows far more than the header fields LibRaw hands the importer (lens
//!   make/model, sub-second and offset times, GPS, serial numbers).
//!
//! Either way the tags the export invalidates are corrected rather than
//! copied: the pixels come out upright and possibly resized, so orientation
//! is reset and the dimensions are the ones actually written.
//!
//! Maker notes are deliberately dropped. They are full of offsets into the
//! original file, so copying the bytes into a re-serialized TIFF structure
//! produces a block that points at nothing.

use exif::experimental::Writer;
use exif::{Field, In, Tag, Value};

use crate::catalog::{FileKind, ImageRecord};

/// Software tag written on everything we export.
const SOFTWARE: &str = "Effective Darkroom";

/// Build the Exif fields for an exported image. `out_w`/`out_h` are the
/// dimensions actually encoded, after any long-edge resize.
pub fn build(rec: &ImageRecord, out_w: u32, out_h: u32) -> Vec<Field> {
    let mut fields = match rec.kind {
        FileKind::Jpeg => from_source_jpeg(&rec.abs_path),
        FileKind::Raw => from_raw(&rec.abs_path),
    };

    // Camera identification, if the source parse did not supply it.
    if !rec.camera_make.is_empty() {
        set_default(&mut fields, Tag::Make, ascii(&rec.camera_make));
    }
    if !rec.camera_model.is_empty() {
        set_default(&mut fields, Tag::Model, ascii(&rec.camera_model));
    }

    // Capture time. The catalog's timestamp is the same camera-local wall
    // clock the importer read, so it is a faithful fallback when the source
    // parse came up empty.
    if let Some(stamp) = rec.capture_ts.and_then(exif_datetime) {
        for tag in [
            Tag::DateTimeOriginal,
            Tag::DateTimeDigitized,
            Tag::DateTime,
        ] {
            set_default(&mut fields, tag, ascii(&stamp));
        }
    }

    // Exif readers treat a missing ExifVersion as "no Exif IFD".
    set_default(
        &mut fields,
        Tag::ExifVersion,
        Value::Undefined(b"0232".to_vec(), 0),
    );

    // What the export itself decides, overriding whatever the source said.
    set(&mut fields, Tag::Orientation, Value::Short(vec![1]));
    set(&mut fields, Tag::PixelXDimension, Value::Long(vec![out_w]));
    set(&mut fields, Tag::PixelYDimension, Value::Long(vec![out_h]));
    set(&mut fields, Tag::Software, ascii(SOFTWARE));
    // The pipeline encodes sRGB.
    set(&mut fields, Tag::ColorSpace, Value::Short(vec![1]));

    fields
}

/// Splice an Exif APP1 segment into an encoded JPEG.
pub fn insert(jpeg: &[u8], fields: &[Field]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        jpeg.starts_with(&[0xFF, 0xD8]),
        "encoder did not produce a JPEG"
    );

    let mut tiff = std::io::Cursor::new(Vec::new());
    let mut writer = Writer::new();
    for field in fields {
        writer.push_field(field);
    }
    writer
        .write(&mut tiff, false)
        .map_err(|e| anyhow::anyhow!("encode exif: {e}"))?;
    let tiff = tiff.into_inner();

    // Segment length counts itself and the "Exif\0\0" header but not the
    // marker, and is a u16.
    let seg_len = 2 + 6 + tiff.len();
    anyhow::ensure!(seg_len <= 0xFFFF, "exif block too large ({seg_len} bytes)");

    // Insert after SOI, and after a leading JFIF APP0 if the encoder wrote
    // one, so the file reads SOI, APP0, APP1 the way other editors write it.
    let mut at = 2;
    if jpeg.len() >= 6 && jpeg[2] == 0xFF && jpeg[3] == 0xE0 {
        let app0_len = u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize;
        if 4 + app0_len <= jpeg.len() {
            at = 4 + app0_len;
        }
    }

    let mut out = Vec::with_capacity(jpeg.len() + seg_len + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(seg_len as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    out.extend_from_slice(&jpeg[at..]);
    Ok(out)
}

/// Tags that must not be carried across from the source.
fn is_stale(tag: Tag) -> bool {
    matches!(
        tag,
        // Offsets into the original file; meaningless once re-serialized.
        Tag::MakerNote
            // Decided by the export.
            | Tag::Orientation
            | Tag::PixelXDimension
            | Tag::PixelYDimension
            | Tag::Software
            | Tag::ColorSpace
            // Describe the source's own storage, not ours.
            | Tag::ImageWidth
            | Tag::ImageLength
            | Tag::Compression
            | Tag::StripOffsets
            | Tag::StripByteCounts
            | Tag::RowsPerStrip
            | Tag::JPEGInterchangeFormat
            | Tag::JPEGInterchangeFormatLength
    )
}

/// Every primary-IFD field of a source JPEG that is still true of the export.
fn from_source_jpeg(path: &std::path::Path) -> Vec<Field> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut reader = std::io::BufReader::new(file);
    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        // No Exif at all (or unreadable): the catalog fallbacks still apply.
        Err(_) => return Vec::new(),
    };
    exif.fields()
        // The thumbnail IFD describes an embedded thumbnail we do not write.
        .filter(|f| f.ifd_num == In::PRIMARY)
        .filter(|f| !is_stale(f.tag))
        // The writer cannot serialize a type it could not parse.
        .filter(|f| !matches!(f.value, Value::Unknown(..)))
        .cloned()
        .collect()
}

/// Reconstruct Exif for a RAW source from rawler's parse.
fn from_raw(path: &std::path::Path) -> Vec<Field> {
    let mut fields = Vec::new();
    let Ok(source) = rawler::rawsource::RawSource::new(path) else {
        return fields;
    };
    let Ok(decoder) = rawler::get_decoder(&source) else {
        return fields;
    };
    let Ok(md) = decoder.raw_metadata(&source, &Default::default()) else {
        return fields;
    };
    let e = &md.exif;

    if !md.make.is_empty() {
        push(&mut fields, Tag::Make, ascii(&md.make));
    }
    if !md.model.is_empty() {
        push(&mut fields, Tag::Model, ascii(&md.model));
    }

    let mut text = |tag, value: &Option<String>| {
        if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
            push(&mut fields, tag, ascii(v));
        }
    };
    text(Tag::DateTimeOriginal, &e.date_time_original);
    text(Tag::DateTimeDigitized, &e.create_date);
    text(Tag::DateTime, &e.modify_date);
    text(Tag::OffsetTime, &e.offset_time);
    text(Tag::OffsetTimeOriginal, &e.offset_time_original);
    text(Tag::OffsetTimeDigitized, &e.offset_time_digitized);
    text(Tag::SubSecTime, &e.sub_sec_time);
    text(Tag::SubSecTimeOriginal, &e.sub_sec_time_original);
    text(Tag::SubSecTimeDigitized, &e.sub_sec_time_digitized);
    text(Tag::LensMake, &e.lens_make);
    text(Tag::LensModel, &e.lens_model);
    text(Tag::LensSerialNumber, &e.lens_serial_number);
    text(Tag::BodySerialNumber, &e.serial_number);
    text(Tag::CameraOwnerName, &e.owner_name);
    text(Tag::Artist, &e.artist);
    text(Tag::Copyright, &e.copyright);

    let mut rational = |tag, value: &Option<rawler::formats::tiff::Rational>| {
        if let Some(r) = value {
            push(&mut fields, tag, Value::Rational(vec![rat(r)]));
        }
    };
    rational(Tag::ExposureTime, &e.exposure_time);
    rational(Tag::FNumber, &e.fnumber);
    rational(Tag::ApertureValue, &e.aperture_value);
    rational(Tag::MaxApertureValue, &e.max_aperture_value);
    rational(Tag::FocalLength, &e.focal_length);
    rational(Tag::SubjectDistance, &e.subject_distance);
    rational(Tag::FlashEnergy, &e.flash_energy);

    let mut srational = |tag, value: &Option<rawler::formats::tiff::SRational>| {
        if let Some(r) = value {
            push(&mut fields, tag, Value::SRational(vec![srat(r)]));
        }
    };
    srational(Tag::ExposureBiasValue, &e.exposure_bias);
    srational(Tag::ShutterSpeedValue, &e.shutter_speed_value);
    srational(Tag::BrightnessValue, &e.brightness_value);

    let mut short = |tag, value: Option<u16>| {
        if let Some(v) = value {
            push(&mut fields, tag, Value::Short(vec![v]));
        }
    };
    short(Tag::PhotographicSensitivity, e.iso_speed_ratings);
    short(Tag::SensitivityType, e.sensitivity_type);
    short(Tag::ExposureProgram, e.exposure_program);
    short(Tag::MeteringMode, e.metering_mode);
    short(Tag::LightSource, e.light_source);
    short(Tag::Flash, e.flash);
    short(Tag::ExposureMode, e.exposure_mode);
    short(Tag::WhiteBalance, e.white_balance);
    short(Tag::SceneCaptureType, e.scene_capture_type);
    short(Tag::SubjectDistanceRange, e.subject_distance_range);

    if let Some(v) = e.iso_speed {
        push(&mut fields, Tag::ISOSpeed, Value::Long(vec![v]));
    }
    if let Some(v) = e.recommended_exposure_index {
        push(&mut fields, Tag::RecommendedExposureIndex, Value::Long(vec![v]));
    }
    if let Some(spec) = &e.lens_spec {
        push(
            &mut fields,
            Tag::LensSpecification,
            Value::Rational(spec.iter().map(rat).collect()),
        );
    }
    if let Some(c) = e.user_comment.as_ref().filter(|c| !c.is_empty()) {
        // UserComment is Undefined with an 8-byte character-code prefix.
        let mut bytes = b"ASCII\0\0\0".to_vec();
        bytes.extend_from_slice(c.as_bytes());
        push(&mut fields, Tag::UserComment, Value::Undefined(bytes, 0));
    }

    if let Some(gps) = &e.gps {
        push_gps(&mut fields, gps);
    }
    fields
}

/// The core location tags. The rest of rawler's GPS block (speed, track,
/// datum) describes how the fix was taken rather than where, and the two
/// cameras this app targets write none of it.
fn push_gps(fields: &mut Vec<Field>, gps: &rawler::exif::ExifGPS) {
    if let Some(v) = gps.gps_version_id {
        push(fields, Tag::GPSVersionID, Value::Byte(v.to_vec()));
    }
    let mut text = |tag, value: &Option<String>| {
        if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
            push(fields, tag, ascii(v));
        }
    };
    text(Tag::GPSLatitudeRef, &gps.gps_latitude_ref);
    text(Tag::GPSLongitudeRef, &gps.gps_longitude_ref);
    let mut triple = |tag, value: &Option<[rawler::formats::tiff::Rational; 3]>| {
        if let Some(v) = value {
            push(fields, tag, Value::Rational(v.iter().map(rat).collect()));
        }
    };
    triple(Tag::GPSLatitude, &gps.gps_latitude);
    triple(Tag::GPSLongitude, &gps.gps_longitude);
    triple(Tag::GPSTimeStamp, &gps.gps_timestamp);
    if let Some(v) = gps.gps_altitude_ref {
        push(fields, Tag::GPSAltitudeRef, Value::Byte(vec![v]));
    }
    if let Some(v) = &gps.gps_altitude {
        push(fields, Tag::GPSAltitude, Value::Rational(vec![rat(v)]));
    }
    if let Some(v) = gps.gps_date_stamp.as_ref().filter(|v| !v.is_empty()) {
        push(fields, Tag::GPSDateStamp, ascii(v));
    }
}

fn rat(r: &rawler::formats::tiff::Rational) -> exif::Rational {
    exif::Rational { num: r.n, denom: r.d }
}

fn srat(r: &rawler::formats::tiff::SRational) -> exif::SRational {
    exif::SRational { num: r.n, denom: r.d }
}

fn ascii(s: &str) -> Value {
    Value::Ascii(vec![s.trim().as_bytes().to_vec()])
}

/// Camera-local wall clock in Exif's "YYYY:MM:DD HH:MM:SS" form. The catalog
/// stores capture times as unix seconds for the local zone (what LibRaw and
/// the JPEG importer both produce), so reading them back locally round-trips.
fn exif_datetime(ts: i64) -> Option<String> {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .earliest()
        .map(|dt| dt.format("%Y:%m:%d %H:%M:%S").to_string())
}

fn push(fields: &mut Vec<Field>, tag: Tag, value: Value) {
    fields.push(Field { tag, ifd_num: In::PRIMARY, value });
}

/// Add a field only if the source did not already provide it.
fn set_default(fields: &mut Vec<Field>, tag: Tag, value: Value) {
    if !fields.iter().any(|f| f.tag == tag && f.ifd_num == In::PRIMARY) {
        push(fields, tag, value);
    }
}

/// Add a field, replacing any the source provided. The writer rejects
/// duplicates, so this cannot simply append.
fn set(fields: &mut Vec<Field>, tag: Tag, value: Value) {
    fields.retain(|f| !(f.tag == tag && f.ifd_num == In::PRIMARY));
    push(fields, tag, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with(fields: &[Field]) -> Vec<u8> {
        // A minimal JFIF-prefixed JPEG skeleton: SOI, APP0, then a body we
        // only need to see preserved.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0xDE, 0xAD, 0xFF, 0xD9]);
        insert(&jpeg, fields).expect("insert exif")
    }

    fn read_back(jpeg: &[u8]) -> exif::Exif {
        exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(jpeg))
            .expect("read exif back")
    }

    fn ascii_of(exif: &exif::Exif, tag: Tag) -> String {
        exif.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    }

    /// The bytes actually stored, rather than `display_value`'s prettified
    /// rendering (which reformats Exif datetimes with dashes).
    fn raw_ascii(exif: &exif::Exif, tag: Tag) -> String {
        match exif.get_field(tag, In::PRIMARY).map(|f| &f.value) {
            Some(Value::Ascii(v)) => {
                String::from_utf8_lossy(v.first().map(|s| s.as_slice()).unwrap_or(b"")).into_owned()
            }
            _ => String::new(),
        }
    }

    #[test]
    fn app1_lands_after_the_jfif_segment_and_keeps_the_body() {
        let out = jpeg_with(&[Field {
            tag: Tag::Software,
            ifd_num: In::PRIMARY,
            value: ascii(SOFTWARE),
        }]);
        assert_eq!(&out[..2], &[0xFF, 0xD8], "SOI first");
        assert_eq!(&out[2..4], &[0xFF, 0xE0], "JFIF APP0 kept in place");
        assert_eq!(&out[8..10], &[0xFF, 0xE1], "APP1 follows APP0");
        // Scan data and EOI survive.
        assert_eq!(&out[out.len() - 6..], &[0xFF, 0xDA, 0xDE, 0xAD, 0xFF, 0xD9]);
    }

    #[test]
    fn round_trips_capture_time_and_camera() {
        let fields = vec![
            Field { tag: Tag::Make, ifd_num: In::PRIMARY, value: ascii("Canon") },
            Field { tag: Tag::Model, ifd_num: In::PRIMARY, value: ascii("EOS RP") },
            Field {
                tag: Tag::DateTimeOriginal,
                ifd_num: In::PRIMARY,
                value: ascii("2026:04:05 14:20:06"),
            },
            Field {
                tag: Tag::ExposureTime,
                ifd_num: In::PRIMARY,
                value: Value::Rational(vec![exif::Rational { num: 1, denom: 250 }]),
            },
        ];
        let exif = read_back(&jpeg_with(&fields));
        assert_eq!(ascii_of(&exif, Tag::Make), "Canon");
        assert_eq!(ascii_of(&exif, Tag::Model), "EOS RP");
        assert_eq!(raw_ascii(&exif, Tag::DateTimeOriginal), "2026:04:05 14:20:06");
        assert_eq!(ascii_of(&exif, Tag::ExposureTime), "1/250");
    }

    #[test]
    fn gps_fields_reach_the_gps_ifd() {
        let fields = vec![
            Field {
                tag: Tag::GPSLatitudeRef,
                ifd_num: In::PRIMARY,
                value: ascii("N"),
            },
            Field {
                tag: Tag::GPSLatitude,
                ifd_num: In::PRIMARY,
                value: Value::Rational(vec![
                    exif::Rational { num: 51, denom: 1 },
                    exif::Rational { num: 30, denom: 1 },
                    exif::Rational { num: 0, denom: 1 },
                ]),
            },
        ];
        let exif = read_back(&jpeg_with(&fields));
        assert_eq!(ascii_of(&exif, Tag::GPSLatitudeRef), "N");
        assert!(exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_some());
    }

    #[test]
    fn overrides_replace_rather_than_duplicate() {
        let mut fields = vec![
            Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) },
            Field { tag: Tag::Software, ifd_num: In::PRIMARY, value: ascii("Some Camera 1.0") },
        ];
        set(&mut fields, Tag::Orientation, Value::Short(vec![1]));
        set(&mut fields, Tag::Software, ascii(SOFTWARE));
        assert_eq!(fields.iter().filter(|f| f.tag == Tag::Orientation).count(), 1);
        let exif = read_back(&jpeg_with(&fields));
        assert_eq!(ascii_of(&exif, Tag::Orientation), "row 0 at top and column 0 at left");
        assert_eq!(ascii_of(&exif, Tag::Software), SOFTWARE);
    }

    #[test]
    fn set_default_yields_to_the_source() {
        let mut fields = vec![Field {
            tag: Tag::Make,
            ifd_num: In::PRIMARY,
            value: ascii("FUJIFILM"),
        }];
        set_default(&mut fields, Tag::Make, ascii("from catalog"));
        set_default(&mut fields, Tag::Model, ascii("X100V"));
        assert_eq!(fields.len(), 2);
        let exif = read_back(&jpeg_with(&fields));
        assert_eq!(ascii_of(&exif, Tag::Make), "FUJIFILM");
        assert_eq!(ascii_of(&exif, Tag::Model), "X100V");
    }

    #[test]
    fn capture_time_round_trips_through_the_local_zone() {
        use chrono::{Local, TimeZone};
        // The catalog stores camera-local wall time as unix seconds, so the
        // clock the importer read has to come back out unchanged.
        let wall = chrono::NaiveDate::from_ymd_opt(2026, 4, 5)
            .unwrap()
            .and_hms_opt(14, 20, 6)
            .unwrap();
        let ts = Local.from_local_datetime(&wall).earliest().unwrap().timestamp();
        assert_eq!(exif_datetime(ts).unwrap(), "2026:04:05 14:20:06");
    }
}
