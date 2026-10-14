use serde::Serialize;
use std::{fs, path::Path, time::UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use crate::{installation_dir, paths};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GalleryImage {
    path: String,
    file_name: String,
    kind: &'static str,
    modified_at: u64,
}

fn scan_directory(directory: &Path, kind: &'static str, images: &mut Vec<GalleryImage>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(file_type) = entry.file_type() else { continue; };
        // Do not follow links out of the package's output directory.
        if file_type.is_symlink() { continue; }
        let path = entry.path();
        if file_type.is_dir() {
            scan_directory(&path, kind, images)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str())
                .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp")) {
            // A generated file may disappear while WebUI is replacing it.
            let Ok(metadata) = entry.metadata() else { continue; };
            let modified_at = metadata.modified().ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_millis() as u64);
            images.push(GalleryImage {
                path: paths::subprocess_path(&path).to_string_lossy().into_owned(),
                file_name: entry.file_name().to_string_lossy().into_owned(),
                kind,
                modified_at,
            });
        }
    }
    Ok(())
}

fn list_images(app: AppHandle) -> Result<Vec<GalleryImage>, String> {
    let outputs = installation_dir(&app)?.join("sd-webui").join("outputs");
    let mut images = Vec::new();
    for (folder, kind) in [("txt2img-images", "txt2img"), ("img2img-images", "img2img")] {
        let directory = outputs.join(folder);
        if !directory.is_dir() { continue; }
        // The installation folder can be on any drive, so a static asset scope is insufficient.
        let directory = paths::subprocess_path(&directory.canonicalize().map_err(|error| error.to_string())?);
        scan_directory(&directory, kind, &mut images)?;
        app.asset_protocol_scope().allow_directory(&directory, true).map_err(|error| error.to_string())?;
    }
    images.sort_unstable_by(|left, right| right.modified_at.cmp(&left.modified_at).then_with(|| left.path.cmp(&right.path)));
    Ok(images)
}

#[tauri::command]
pub(crate) async fn list_gallery_images(app: AppHandle) -> Result<Vec<GalleryImage>, String> {
    tauri::async_runtime::spawn_blocking(move || list_images(app)).await.map_err(|error| error.to_string())?
}
