//! Immich integration: upload rendered JPEGs as assets over the HTTP API
//! (x-api-key auth) and file them into a new or existing album.

use anyhow::{anyhow, Context, Result};
use chrono::TimeZone;
use serde::Deserialize;

use crate::catalog::{Catalog, ImageRecord};
use crate::engine::export::{
    export_file_name, render_full, resize_long_edge, ExportOutcome, Naming,
};
use crate::engine::RenderEngine;

/// Where uploaded assets should be filed.
#[derive(Debug, Clone)]
pub enum AlbumTarget {
    None,
    /// Create a new album with this name holding the uploaded assets.
    New(String),
    /// Add the uploaded assets to an existing album.
    Existing { id: String, name: String },
}

#[derive(Debug, Clone)]
pub struct ImmichExportSpec {
    pub image_ids: Vec<i64>,
    pub url: String,
    pub api_key: String,
    /// JPEG quality 1-100.
    pub quality: u8,
    /// Resize so the long edge is at most this many pixels.
    pub long_edge: Option<u32>,
    /// Optional filename prefix.
    pub prefix: String,
    pub naming: Naming,
    pub album: AlbumTarget,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Album {
    pub id: String,
    #[serde(rename = "albumName")]
    pub name: String,
    #[serde(rename = "assetCount", default)]
    pub asset_count: u32,
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    api_key: String,
}

/// Accept the server URL with or without a trailing slash or `/api` suffix.
fn normalize_url(url: &str) -> String {
    let mut s = url.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix("/api") {
        s = stripped.trim_end_matches('/').to_string();
    }
    s
}

/// Immich errors come as JSON `{"message": ...}`; fall back to the raw body.
fn error_detail(body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").map(|m| m.to_string()))
        .unwrap_or_else(|| body.to_string());
    let mut msg = msg.trim().to_string();
    if msg.len() > 200 {
        msg.truncate(200);
        msg.push('…');
    }
    msg
}

fn read_ok(mut res: ureq::http::Response<ureq::Body>) -> Result<ureq::Body> {
    let status = res.status();
    if status.is_success() {
        return Ok(res.into_body());
    }
    let detail = res.body_mut().read_to_string().unwrap_or_default();
    Err(anyhow!("HTTP {}: {}", status.as_u16(), error_detail(&detail)))
}

impl Client {
    pub fn new(url: &str, api_key: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .build()
            .new_agent();
        Self {
            agent,
            base: normalize_url(url),
            api_key: api_key.trim().to_string(),
        }
    }

    /// Validate the URL + key; returns the account it authenticates as.
    pub fn me(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct Me {
            name: String,
            email: String,
        }
        let res = self
            .agent
            .get(format!("{}/api/users/me", self.base))
            .header("x-api-key", self.api_key.as_str())
            .call()
            .context("connect to Immich")?;
        let me: Me = read_ok(res)?.read_json().context("parse account info")?;
        Ok(format!("{} ({})", me.name, me.email))
    }

    pub fn albums(&self) -> Result<Vec<Album>> {
        let res = self
            .agent
            .get(format!("{}/api/albums", self.base))
            .header("x-api-key", self.api_key.as_str())
            .call()
            .context("connect to Immich")?;
        let mut albums: Vec<Album> =
            read_ok(res)?.read_json().context("parse album list")?;
        albums.sort_by_key(|a| a.name.to_lowercase());
        Ok(albums)
    }

    /// Upload one JPEG; returns the asset id (an existing id if the server
    /// already has this file, so album filing still works for duplicates).
    pub fn upload(
        &self,
        device_asset_id: &str,
        file_name: &str,
        jpeg: &[u8],
        created_iso: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Uploaded {
            id: String,
        }
        const BOUNDARY: &str = "----effective-darkroom-2c9f04d1a7";
        let mut body = Vec::with_capacity(jpeg.len() + 1024);
        for (name, value) in [
            ("deviceAssetId", device_asset_id),
            ("deviceId", "effective-darkroom"),
            ("fileCreatedAt", created_iso),
            ("fileModifiedAt", created_iso),
        ] {
            body.extend_from_slice(
                format!(
                    "--{BOUNDARY}\r\nContent-Disposition: form-data; \
                     name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; \
                 name=\"assetData\"; filename=\"{file_name}\"\r\n\
                 Content-Type: image/jpeg\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(jpeg);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        let res = self
            .agent
            .post(format!("{}/api/assets", self.base))
            .header("x-api-key", self.api_key.as_str())
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .send(&body[..])
            .context("upload to Immich")?;
        let up: Uploaded = read_ok(res)?.read_json().context("parse upload response")?;
        Ok(up.id)
    }

    pub fn create_album(&self, name: &str, asset_ids: &[String]) -> Result<String> {
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        let res = self
            .agent
            .post(format!("{}/api/albums", self.base))
            .header("x-api-key", self.api_key.as_str())
            .send_json(serde_json::json!({ "albumName": name, "assetIds": asset_ids }))
            .context("create album")?;
        let created: Created =
            read_ok(res)?.read_json().context("parse create-album response")?;
        Ok(created.id)
    }

    pub fn add_to_album(&self, album_id: &str, asset_ids: &[String]) -> Result<()> {
        let res = self
            .agent
            .put(format!("{}/api/albums/{album_id}/assets", self.base))
            .header("x-api-key", self.api_key.as_str())
            .send_json(serde_json::json!({ "ids": asset_ids }))
            .context("add assets to album")?;
        read_ok(res)?;
        Ok(())
    }
}

/// Run a full Immich export synchronously (call from a worker thread).
/// `progress` receives (index, total, file name) before each image.
pub fn run_blocking(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    spec: &ImmichExportSpec,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<ExportOutcome> {
    let client = Client::new(&spec.url, &spec.api_key);
    let all = catalog.all_images()?;
    let total = spec.image_ids.len();
    let mut outcome = ExportOutcome::default();
    let mut asset_ids = Vec::new();

    for (i, id) in spec.image_ids.iter().enumerate() {
        let Some(rec) = all.iter().find(|r| r.id == *id) else {
            outcome.errors.push(format!("image #{id} not in catalog"));
            continue;
        };
        progress(i + 1, total, &rec.file_name);
        match upload_one(engine, catalog, &client, spec, rec) {
            Ok(asset_id) => {
                outcome.exported += 1;
                asset_ids.push(asset_id);
            }
            Err(e) => outcome.errors.push(format!("{}: {e:#}", rec.file_name)),
        }
    }

    if !asset_ids.is_empty() {
        let filed = match &spec.album {
            AlbumTarget::None => Ok(()),
            AlbumTarget::New(name) => client.create_album(name, &asset_ids).map(|_| ()),
            AlbumTarget::Existing { id, .. } => client.add_to_album(id, &asset_ids),
        };
        if let Err(e) = filed {
            outcome.errors.push(format!("album: {e:#}"));
        }
    }
    Ok(outcome)
}

fn upload_one(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    client: &Client,
    spec: &ImmichExportSpec,
    rec: &ImageRecord,
) -> Result<String> {
    let rendered = render_full(engine, catalog, rec)?;
    let final_img = match spec.long_edge {
        Some(edge) if edge > 0 && edge < rendered.width().max(rendered.height()) => {
            resize_long_edge(&rendered, edge)?
        }
        _ => rendered,
    };

    let mut jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        std::io::Cursor::new(&mut jpeg),
        spec.quality.clamp(1, 100),
    );
    let rgb = image::DynamicImage::ImageRgba8(final_img).to_rgb8();
    encoder
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .context("encode jpeg")?;

    let name = export_file_name(&spec.prefix, spec.naming, "jpg", rec);
    let created = chrono::Utc
        .timestamp_opt(rec.capture_ts.unwrap_or(rec.imported_ts), 0)
        .earliest()
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    client.upload(&format!("effective-darkroom-{}-{name}", rec.id), &name, &jpeg, &created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalization() {
        assert_eq!(normalize_url("https://photos.example.com"), "https://photos.example.com");
        assert_eq!(normalize_url(" https://photos.example.com/ "), "https://photos.example.com");
        assert_eq!(normalize_url("https://photos.example.com/api/"), "https://photos.example.com");
        assert_eq!(normalize_url("http://10.0.0.5:2283/api"), "http://10.0.0.5:2283");
    }

    #[test]
    fn error_detail_extracts_message() {
        assert_eq!(error_detail(r#"{"message":"Invalid API key"}"#), "\"Invalid API key\"");
        assert_eq!(error_detail("plain failure"), "plain failure");
    }
}
