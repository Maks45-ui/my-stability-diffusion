use serde::Serialize;
use std::{collections::HashSet, fs, path::{Path, PathBuf}, time::UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{package_dir, paths};

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
    let package = package_dir(&app)?;
    let mut images = Vec::new();
    for (folder, kind) in [("txt2img-images", "txt2img"), ("img2img-images", "img2img")] {
        for root in ["outputs", "output"] {
            let directory = package.join(root).join(folder);
            if !directory.is_dir() { continue; }
            // The installation folder can be on any drive, so a static asset scope is insufficient.
            let directory = paths::subprocess_path(&directory.canonicalize().map_err(|error| error.to_string())?);
            scan_directory(&directory, kind, &mut images)?;
            app.asset_protocol_scope().allow_directory(&directory, true).map_err(|error| error.to_string())?;
        }
    }
    images.sort_unstable_by(|left, right| right.modified_at.cmp(&left.modified_at).then_with(|| left.path.cmp(&right.path)));
    Ok(images)
}

#[tauri::command]
pub(crate) async fn list_gallery_images(app: AppHandle) -> Result<Vec<GalleryImage>, String> {
    tauri::async_runtime::spawn_blocking(move || list_images(app)).await.map_err(|error| error.to_string())?
}

fn gallery_roots(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let package = package_dir(app)?;
    let mut roots = Vec::new();
    for parent in ["outputs", "output"] {
        for folder in ["txt2img-images", "img2img-images"] {
            let directory = package.join(parent).join(folder);
            if directory.is_dir() {
                roots.push(directory.canonicalize().map_err(|error| error.to_string())?);
            }
        }
    }
    Ok(roots)
}

fn checked_image(path: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        return Err("Путь изображения должен быть абсолютным.".to_owned());
    }
    if !candidate.extension().and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp")) {
        return Err("Можно выбрать только изображения PNG, JPG или WebP.".to_owned());
    }
    let metadata = fs::symlink_metadata(candidate).map_err(|error| format!("Изображение не найдено: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Выбранный путь не является обычным файлом изображения.".to_owned());
    }
    let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
    if !roots.iter().any(|root| canonical.starts_with(root) && canonical.as_path() != root.as_path()) {
        return Err("Файл находится вне папок результатов активного WebUI.".to_owned());
    }
    Ok(canonical)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GalleryImageDetails {
    prompt: Option<String>,
    negative_prompt: Option<String>,
    seed: Option<String>,
    steps: Option<String>,
    sampler: Option<String>,
    model: Option<String>,
    parameters: Option<String>,
}

fn parse_parameters(raw: Option<String>) -> GalleryImageDetails {
    let mut details = GalleryImageDetails {
        prompt: None, negative_prompt: None, seed: None, steps: None,
        sampler: None, model: None, parameters: raw.clone(),
    };
    let Some(text) = raw else { return details; };
    let settings_start = text.rfind("\nSteps: ").map(|index| index + 1)
        .or_else(|| text.starts_with("Steps: ").then_some(0));
    let (body, settings) = if let Some(index) = settings_start {
        (&text[..index], &text[index..])
    } else { (text.as_str(), "") };
    let (prompt, negative) = if let Some(index) = body.find("Negative prompt: ") {
        (&body[..index], Some(body[index + "Negative prompt: ".len()..].trim()))
    } else { (body, None) };
    let prompt = prompt.trim();
    if !prompt.is_empty() { details.prompt = Some(prompt.to_owned()); }
    details.negative_prompt = negative.filter(|value| !value.is_empty()).map(str::to_owned);
    for part in settings.split(", ") {
        if let Some((key, value)) = part.split_once(": ") {
            let value = value.trim().to_owned();
            match key {
                "Seed" => details.seed = Some(value),
                "Steps" => details.steps = Some(value),
                "Sampler" => details.sampler = Some(value),
                "Model" => details.model = Some(value),
                _ => {}
            }
        }
    }
    details
}

fn png_parameters(file: &Path) -> Result<Option<String>, String> {
    if !file.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("png")) {
        return Ok(None);
    }
    let input = fs::File::open(file).map_err(|error| error.to_string())?;
    let decoder = png::Decoder::new(std::io::BufReader::new(input));
    let reader = decoder.read_info().map_err(|error| format!("Не удалось прочитать PNG: {error}"))?;
    let info = reader.info();
    for chunk in &info.uncompressed_latin1_text {
        if chunk.keyword == "parameters" { return Ok(Some(chunk.text.clone())); }
    }
    for chunk in &info.compressed_latin1_text {
        if chunk.keyword == "parameters" {
            return chunk.get_text().map(Some).map_err(|error| error.to_string());
        }
    }
    for chunk in &info.utf8_text {
        if chunk.keyword == "parameters" {
            return chunk.get_text().map(Some).map_err(|error| error.to_string());
        }
    }
    Ok(None)
}

#[tauri::command]
pub(crate) async fn get_gallery_image_details(app: AppHandle, path: String) -> Result<GalleryImageDetails, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = checked_image(&path, &gallery_roots(&app)?)?;
        png_parameters(&file).map(parse_parameters)
    }).await.map_err(|error| error.to_string())?
}

#[cfg(windows)]
fn preserve_original_png_in_clipboard(file: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    };

    let bytes = fs::read(file).map_err(|error| format!("Не удалось прочитать PNG: {error}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Файл не является корректным PNG.".to_owned());
    }
    // Chromium's PNG clipboard format retains tEXt chunks used by WebUI PNG info.
    // The clipboard plugin has already written CF_DIB for ordinary image pastes.
    unsafe {
        let format = RegisterClipboardFormatW([b'P' as u16, b'N' as u16, b'G' as u16, 0].as_ptr());
        if format == 0 {
            return Err(format!("Не удалось зарегистрировать формат PNG: {}", std::io::Error::last_os_error()));
        }
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if handle.is_null() {
            return Err(format!("Не удалось выделить память для PNG: {}", std::io::Error::last_os_error()));
        }
        let destination = GlobalLock(handle);
        if destination.is_null() {
            GlobalFree(handle);
            return Err(format!("Не удалось подготовить PNG: {}", std::io::Error::last_os_error()));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast::<u8>(), bytes.len());
        GlobalUnlock(handle);
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            GlobalFree(handle);
            return Err(format!("Буфер обмена занят: {}", std::io::Error::last_os_error()));
        }
        // Add the original PNG alongside CF_DIB. Do not call EmptyClipboard here.
        let result = SetClipboardData(format, handle);
        let error = if result.is_null() { Some(std::io::Error::last_os_error()) } else { None };
        CloseClipboard();
        if let Some(error) = error {
            GlobalFree(handle);
            return Err(format!("Не удалось сохранить метаданные PNG в буфере обмена: {error}"));
        }
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn copy_gallery_image(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = checked_image(&path, &gallery_roots(&app)?)?;
        let size = fs::metadata(&file).map_err(|error| error.to_string())?.len();
        if size > 100 * 1024 * 1024 {
            return Err("Изображение слишком велико для копирования.".to_owned());
        }
        let decoded = image::open(&file).map_err(|error| format!("Не удалось прочитать изображение: {error}"))?;
        let rgba = decoded.into_rgba8();
        let (width, height) = rgba.dimensions();
        let clipboard_image = tauri::image::Image::new_owned(rgba.into_raw(), width, height);
        app.clipboard().write_image(&clipboard_image).map_err(|error| format!("Не удалось скопировать изображение: {error}"))?;
        #[cfg(windows)]
        if file.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("png")) {
            preserve_original_png_in_clipboard(&file)?;
        }
        Ok(())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn delete_gallery_images(app: AppHandle, paths: Vec<String>) -> Result<usize, String> {
    if paths.is_empty() || paths.len() > 500 {
        return Err("Выберите от 1 до 500 изображений для удаления.".to_owned());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let roots = gallery_roots(&app)?;
        let mut unique = HashSet::new();
        let mut files = Vec::new();
        // Validate the complete batch before removing the first file.
        for path in paths {
            let file = checked_image(&path, &roots)?;
            if unique.insert(file.clone()) { files.push(file); }
        }
        let mut deleted = 0;
        for file in files {
            fs::remove_file(&file).map_err(|error| format!("Удалено {deleted} изображений; не удалось удалить {}: {error}", file.display()))?;
            deleted += 1;
        }
        Ok(deleted)
    }).await.map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::{checked_image, parse_parameters};
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn parses_a1111_generation_metadata() {
        let details = parse_parameters(Some("a portrait\nNegative prompt: blurry\nSteps: 28, Sampler: Euler a, Seed: 42, Model: test".to_owned()));
        assert_eq!(details.prompt.as_deref(), Some("a portrait"));
        assert_eq!(details.negative_prompt.as_deref(), Some("blurry"));
        assert_eq!(details.steps.as_deref(), Some("28"));
        assert_eq!(details.seed.as_deref(), Some("42"));
        assert_eq!(details.model.as_deref(), Some("test"));
    }

    #[test]
    fn deletion_scope_accepts_only_images_inside_output_root() {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("flow-gallery-{}-{unique}", std::process::id()));
        let root = base.join("outputs/txt2img-images");
        fs::create_dir_all(root.join("2026-10-23")).unwrap();
        let inside = root.join("2026-10-23/image.png");
        let outside = base.join("elsewhere.png");
        fs::write(&inside, b"test").unwrap();
        fs::write(&outside, b"test").unwrap();
        let roots = vec![root.canonicalize().unwrap()];
        assert_eq!(checked_image(&inside.to_string_lossy(), &roots).unwrap(), inside.canonicalize().unwrap());
        assert!(checked_image(&outside.to_string_lossy(), &roots).is_err());
        assert!(checked_image(&root.join("2026-10-23/notes.txt").to_string_lossy(), &roots).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}