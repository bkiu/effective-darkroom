//! Import from USB-connected cameras (PTP via libgphoto2).

use std::path::Path;

use anyhow::{Context as _, Result};
use gphoto2::{Camera, Context};

use crate::catalog::FileKind;

#[derive(Debug, Clone)]
pub struct CameraDesc {
    pub model: String,
    pub port: String,
}

/// List currently connected cameras.
pub fn detect() -> Result<Vec<CameraDesc>> {
    let ctx = Context::new().context("init gphoto2")?;
    let cameras = ctx.list_cameras().wait().context("list cameras")?;
    Ok(cameras
        .map(|d| CameraDesc {
            model: d.model,
            port: d.port,
        })
        .collect())
}

pub fn connect(desc: &CameraDesc) -> Result<Camera> {
    let ctx = Context::new().context("init gphoto2")?;
    ctx.get_camera(&gphoto2::list::CameraDescriptor {
        model: desc.model.clone(),
        port: desc.port.clone(),
    })
    .wait()
    .with_context(|| format!("connect to {}", desc.model))
}

/// An importable file on the camera card.
#[derive(Debug, Clone)]
pub struct CardFile {
    pub folder: String,
    pub name: String,
}

/// Recursively list importable files (CR3/RAF/JPG) on all card folders.
pub fn list_card_files(camera: &Camera) -> Result<Vec<CardFile>> {
    let fs = camera.fs();
    let mut out = Vec::new();
    let mut stack = vec![String::from("/")];
    while let Some(folder) = stack.pop() {
        for name in fs
            .list_files(&folder)
            .wait()
            .with_context(|| format!("list files in {folder}"))?
        {
            if FileKind::from_path(Path::new(&name)).is_some() {
                out.push(CardFile {
                    folder: folder.clone(),
                    name,
                });
            }
        }
        for sub in fs
            .list_folders(&folder)
            .wait()
            .with_context(|| format!("list folders in {folder}"))?
        {
            if folder == "/" {
                stack.push(format!("/{sub}"));
            } else {
                stack.push(format!("{folder}/{sub}"));
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Size in bytes as reported by the camera, if available.
pub fn file_size(camera: &Camera, file: &CardFile) -> Option<u64> {
    camera
        .fs()
        .file_info(&file.folder, &file.name)
        .wait()
        .ok()
        .and_then(|info| info.file().size())
}

pub fn download(camera: &Camera, file: &CardFile, dest: &Path) -> Result<()> {
    camera
        .fs()
        .download_to(&file.folder, &file.name, dest)
        .wait()
        .with_context(|| format!("download {}", file.name))?;
    Ok(())
}
