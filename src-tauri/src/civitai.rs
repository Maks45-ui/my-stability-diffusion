use crate::{civitai_token, emit_task, TaskEvent};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::{Duration, Instant}};
use tauri::{AppHandle, State};
use url::Url;

const API: &str = "https://civitai.red/api/v1/models";

#[derive(Default)]
pub(crate) struct DownloadControl {
    running: AtomicBool,
    cancelled: AtomicBool,
    paused: AtomicBool,
}

impl DownloadControl {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadedFile {
    name: String,
    model_type: String,
    size_bytes: u64,
    downloaded_bytes: u64,
    status: &'static str,
    download_url: Option<String>,
    sha256: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelFile {
    name: String,
    download_url: String,
    size_kb: f64,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialMetadata {
    file: ModelFile,
    model_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelSummary {
    id: u64,
    name: String,
    #[serde(rename = "type")]
    model_type: String,
    description: Option<String>,
    tags: Vec<String>,
    creator: Option<String>,
    download_count: Option<u64>,
    favorite_count: Option<u64>,
    updated_at: Option<String>,
    images: Vec<String>,
    files: Vec<ModelFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StarterModel {
    preview_url: Option<String>,
    file: ModelFile,
}

/// Resolve a pinned Civitai version, never the latest version of a model.
#[tauri::command]
pub(crate) async fn get_starter_model(app: AppHandle, key: String) -> Result<StarterModel, String> {
    let (model_id, version_id) = match key.as_str() {
        "indigo" => (34469, 397050),
        "wai" => (827184, 2883731),
        "realistic" => (4201, 501240),
        _ => return Err("Неизвестная стартовая модель.".to_owned()),
    };
    let url = format!("https://civitai.red/api/v1/model-versions/{version_id}");
    let mut request = client()?.get(url);
    let token = civitai_token(&app)?;
    if !token.is_empty() { request = request.bearer_auth(token); }
    let response = request.send().await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?;
    let version: Value = response.json().await.map_err(|error| error.to_string())?;
    if version.get("id").and_then(Value::as_u64) != Some(version_id)
        || version.get("modelId").and_then(Value::as_u64) != Some(model_id) {
        return Err("Civitai вернул другую версию модели.".to_owned());
    }
    let files = version.get("files").and_then(Value::as_array).ok_or("У версии нет файлов.")?;
    let selected = files.iter()
        .filter(|file| file.get("name").and_then(Value::as_str).is_some_and(|name| name.to_ascii_lowercase().ends_with(".safetensors")))
        .max_by_key(|file| {
            let is_model = file.get("type").and_then(Value::as_str) == Some("Model");
            let primary = file.get("primary").and_then(Value::as_bool) == Some(true);
            (is_model, primary)
        })
        .ok_or("У выбранной версии нет файла SafeTensors.")?;
    let name = safe_name(selected.get("name").and_then(Value::as_str).unwrap_or_default());
    let download_url = selected.get("downloadUrl").and_then(Value::as_str).ok_or("Civitai не вернул ссылку на файл.")?.to_owned();
    if name.is_empty() { return Err("Некорректное имя файла модели.".to_owned()); }
    let url = allowed_url(&download_url)?;
    if !url.path().starts_with("/api/download/models/") { return Err("Недопустимая ссылка на файл модели.".to_owned()); }
    let preview_url = version.get("images").and_then(Value::as_array).and_then(|images| images.iter()
        .filter(|image| image.get("type").and_then(Value::as_str) != Some("video"))
        .filter_map(|image| image.get("url").and_then(Value::as_str))
        .find(|url| allowed_url(url).is_ok()).map(str::to_owned));
    Ok(StarterModel { preview_url, file: ModelFile {
        name, download_url, size_kb: selected.get("sizeKB").and_then(Value::as_f64).unwrap_or(0.0),
        base_model: version.get("baseModel").and_then(Value::as_str).map(str::to_owned),
        sha256: selected.pointer("/hashes/SHA256").and_then(Value::as_str).map(str::to_owned),
    } })
}

fn allowed_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| error.to_string())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if url.scheme() != "https" || !["civitai.com", "civitai.red"].iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))) {
        return Err("Недопустимый адрес Civitai.".to_owned());
    }
    Ok(url)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(45)).build().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn search_models(app: AppHandle, query: String, sort: String, model_type: String) -> Result<Vec<ModelSummary>, String> {
    let trimmed = query.trim();
    let sorting = match sort.as_str() {
        "newest" => "Newest",
        "rated" => "Highest Rated",
        _ => "Most Downloaded",
    };
    let mut request = client()?.get(API).query(&[("limit", "12"), ("nsfw", "true"), ("sort", sorting)]);
    let category = match model_type.as_str() {
        "checkpoint" => Some("Checkpoint"),
        "lora" => Some("LORA"),
        "vae" => Some("VAE"),
        _ => None,
    };
    if let Some(category) = category { request = request.query(&[("types", category)]); }
    if !trimmed.is_empty() {
        request = if trimmed.parse::<u64>().is_ok() { request.query(&[("ids", trimmed)]) } else { request.query(&[("query", trimmed)]) };
    }
    let token = civitai_token(&app)?;
    if !token.is_empty() { request = request.bearer_auth(token); }
    let response = request.send().await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?;
    let root: Value = response.json().await.map_err(|error| error.to_string())?;
    let models = root.get("items").and_then(Value::as_array).ok_or("Некорректный ответ Civitai.")?;
    Ok(models.iter().filter_map(|model| {
        let id = model.get("id")?.as_u64()?;
        let mut images: Vec<String> = Vec::new();
        let mut files: Vec<ModelFile> = Vec::new();
        if let Some(versions) = model.get("modelVersions").and_then(Value::as_array) {
            // Keep every example and file across versions, as in the previous catalog.
            for version in versions {
                if let Some(examples) = version.get("images").and_then(Value::as_array) {
                    for image in examples {
                        if image.get("type").and_then(Value::as_str) == Some("video") { continue; }
                        if let Some(url) = image.get("url").and_then(Value::as_str) {
                            if !images.iter().any(|saved| saved.as_str() == url) { images.push(url.to_owned()); }
                        }
                    }
                }
                if let Some(available) = version.get("files").and_then(Value::as_array) {
                    for item in available {
                        if let (Some(name), Some(download_url)) = (
                            item.get("name").and_then(Value::as_str),
                            item.get("downloadUrl").and_then(Value::as_str),
                        ) {
                            let name = safe_name(name);
                            if name.is_empty() { continue; }
                            files.push(ModelFile {
                                name, download_url: download_url.to_owned(),
                                size_kb: item.get("sizeKB").and_then(Value::as_f64).unwrap_or(0.0),
                                base_model: version.get("baseModel").and_then(Value::as_str).map(str::to_owned),
                                sha256: item.pointer("/hashes/SHA256").and_then(Value::as_str).map(str::to_owned),
                            });
                        }
                    }
                }
            }
        }
        Some(ModelSummary {
            id,
            name: model.get("name").and_then(Value::as_str).unwrap_or("Без названия").to_owned(),
            model_type: model.get("type").and_then(Value::as_str).unwrap_or("Модель").to_owned(),
            description: model.get("description").and_then(Value::as_str)
                .map(|value| value.chars().take(5_000).collect()),
            tags: model.get("tags").and_then(Value::as_array).map(|values| values.iter()
                .filter_map(Value::as_str).take(3).map(str::to_owned).collect()).unwrap_or_default(),
            creator: model.pointer("/creator/username").and_then(Value::as_str).map(str::to_owned),
            download_count: model.pointer("/stats/downloadCount").and_then(Value::as_u64),
            favorite_count: model.pointer("/stats/favoriteCount").or_else(|| model.pointer("/stats/thumbsUpCount")).and_then(Value::as_u64),
            updated_at: model.get("updatedAt").or_else(|| model.pointer("/modelVersions/0/publishedAt")).and_then(Value::as_str).map(str::to_owned),
            images,
            files,
        })
    }).collect())
}

#[tauri::command]
pub(crate) async fn get_preview(app: AppHandle, url: String) -> Result<String, String> {
    let url = allowed_url(&url)?;
    let mut request = client()?.get(url);
    let token = civitai_token(&app)?;
    if !token.is_empty() { request = request.bearer_auth(token); }
    let mut response = request.send().await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?;
    let mime = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("image/jpeg").split(';').next().unwrap_or("image/jpeg").to_owned();
    if !mime.starts_with("image/") { return Err("Ответ не содержит изображение.".to_owned()); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > 12 * 1024 * 1024 { return Err("Превью слишком велико.".to_owned()); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn safe_name(value: &str) -> String {
    value.chars().filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ')).take(150).collect::<String>().trim_matches(|c| c == '.' || c == ' ').to_owned()
}

pub(crate) fn models_directory(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::models_library_dir(app)?.join("Stable-diffusion"))
}

fn model_subfolder(model_type: &str) -> Result<&'static str, String> {
    match model_type.to_ascii_lowercase().as_str() {
        "checkpoint" | "stable-diffusion" => Ok("Stable-diffusion"),
        "lora" | "locon" | "lycoris" | "dora" => Ok("Lora"),
        "vae" => Ok("VAE"),
        _ => Err(format!("Тип модели «{model_type}» пока не поддерживается для установки. Доступны Checkpoint, LoRA и VAE.")),
    }
}

fn paths(app: &AppHandle, model_type: &str, file_name: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let name = safe_name(file_name);
    if name.is_empty() || name != file_name { return Err("Некорректное имя файла модели.".to_owned()); }
    let target = crate::models_library_dir(app)?.join(model_subfolder(model_type)?).join(&name);
    let temp = target.with_file_name(format!("{name}.flow-part"));
    let meta = target.with_file_name(format!("{name}.flow-meta.json"));
    Ok((target, temp, meta))
}

fn expected_sha256(value: Option<&str>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(hash) if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) => Ok(Some(hash.to_ascii_lowercase())),
        Some(_) => Err("Civitai вернул некорректную контрольную сумму SHA256.".to_owned()),
    }
}

fn verify_sha256(path: &std::path::Path, expected: &str) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual == expected { Ok(()) }
    else { Err(format!("SHA256 не совпадает: ожидалось {expected}, получено {actual}. Повреждённая загрузка удалена.")) }
}

fn progress(app: &AppHandle, task_id: &str, file_name: &str, copied: u64, total: u64, speed: Option<f64>) {
    emit_task(app, TaskEvent {
        kind: "download", phase: "dependencies", message: file_name.to_owned(),
        percent: if total > 0 { Some((copied.saturating_mul(100) / total).min(100) as u8) } else { None },
        bytes_per_second: speed, task_id: Some(task_id.to_owned()),
        bytes_downloaded: Some(copied), total_bytes: Some(total), url: None,
    });
}

fn notify_hash_check(app: &AppHandle, task_id: &str, file_name: &str) {
    emit_task(app, TaskEvent {
        kind: "download", phase: "verifying", message: format!("Проверка SHA256: {file_name}"),
        percent: Some(100), bytes_per_second: None, task_id: Some(task_id.to_owned()),
        bytes_downloaded: None, total_bytes: None, url: None,
    });
}

fn download(app: &AppHandle, control: &DownloadControl, file: ModelFile, model_type: String, task_id: &str, resume: bool) -> Result<(), String> {
    let mut url = allowed_url(&file.download_url)?;
    if !url.path().starts_with("/api/download/models/") { return Err("Недопустимая ссылка скачивания.".to_owned()); }
    url.set_host(Some("civitai.red")).map_err(|_| "Недопустимый адрес скачивания.".to_owned())?;
    let (target, temp, meta) = paths(app, &model_type, &file.name)?;
    let expected_hash = expected_sha256(file.sha256.as_deref())?;
    if let Some(folder) = target.parent() { fs::create_dir_all(folder).map_err(|error| error.to_string())?; }
    if target.exists() { return Err("Файл уже загружен.".to_owned()); }
    if resume && !temp.exists() && !meta.exists() && model_subfolder(&model_type)? != "Stable-diffusion" {
        let legacy_target = models_directory(app)?.join(&file.name);
        let legacy_temp = legacy_target.with_file_name(format!("{}.flow-part", file.name));
        let legacy_meta = legacy_target.with_file_name(format!("{}.flow-meta.json", file.name));
        if legacy_temp.is_file() && legacy_meta.is_file() {
            let saved: PartialMetadata = serde_json::from_slice(&fs::read(&legacy_meta).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            if saved.model_type == model_type && saved.file.name == file.name {
                fs::rename(&legacy_temp, &temp).map_err(|error| error.to_string())?;
                fs::rename(&legacy_meta, &meta).map_err(|error| error.to_string())?;
            }
        }
    }

    if !resume && temp.exists() { return Err("Найдена незавершённая загрузка. Удалите её перед повтором.".to_owned()); }
    if resume && temp.exists() && !meta.exists() { return Err("Не найдены сведения для продолжения загрузки.".to_owned()); }
    if resume && meta.exists() {
        let saved: PartialMetadata = serde_json::from_slice(&fs::read(&meta).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if saved.file.name != file.name || saved.file.download_url != file.download_url || saved.model_type != model_type || saved.file.sha256 != file.sha256 {
            return Err("Данные незавершённой загрузки не совпадают с выбранным файлом.".to_owned());
        }
    }
    let mut previous = if resume { fs::metadata(&temp).map(|metadata| metadata.len()).unwrap_or(0) } else { 0 };
    let token = civitai_token(app)?;
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(300)).build().map_err(|error| error.to_string())?;
    let mut request = client.get(url);
    if !token.is_empty() { request = request.bearer_auth(token); }
    if previous > 0 { request = request.header(reqwest::header::RANGE, format!("bytes={previous}-")); }
    let response = request.send().map_err(|error| error.to_string())?;
    if previous > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let prefix = format!("bytes {previous}-");
        let content_range = response.headers().get(reqwest::header::CONTENT_RANGE).and_then(|value| value.to_str().ok()).unwrap_or_default();
        if !content_range.starts_with(&prefix) { return Err("Сервер вернул неверный диапазон файла.".to_owned()); }
    } else if previous > 0 && response.status() == reqwest::StatusCode::OK {
        // The server ignored Range. Restart safely instead of appending a duplicate file.
        previous = 0;
    }
    let mut response = response.error_for_status().map_err(|error| error.to_string())?;
    let mime = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or_default();
    if mime.starts_with("text/html") || mime.starts_with("application/json") { return Err("Civitai вернул страницу вместо файла модели.".to_owned()); }
    let expected = if file.size_kb.is_finite() && file.size_kb > 0.0 { (file.size_kb * 1024.0) as u64 } else { 0 };
    let known_total = response.content_length().map(|length| previous.saturating_add(length));
    let total = known_total.unwrap_or(expected);
    let metadata = PartialMetadata { file: file.clone(), model_type: model_type.clone() };
    fs::write(&meta, serde_json::to_vec(&metadata).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let mut output = OpenOptions::new().create(true).write(true).append(previous > 0).truncate(previous == 0).open(&temp).map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut last_update = Instant::now();
    progress(app, task_id, &file.name, previous, total, None);
    let result = (|| -> Result<(), String> {
        let mut copied = previous;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if control.cancelled.load(Ordering::SeqCst) { return Err("Загрузка отменена.".to_owned()); }
            if control.paused.load(Ordering::SeqCst) { return Err("Загрузка приостановлена.".to_owned()); }
            let count = response.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 { break; }
            output.write_all(&buffer[..count]).map_err(|error| error.to_string())?;
            copied += count as u64;
            if last_update.elapsed() >= Duration::from_millis(150) {
                progress(app, task_id, &file.name, copied, total, Some((copied - previous) as f64 / started.elapsed().as_secs_f64().max(0.1)));
                last_update = Instant::now();
            }
        }
        if copied == 0 { return Err("Получен пустой файл модели.".to_owned()); }
        if known_total.is_some() && copied != total { return Err("Загрузка прервалась до получения всего файла.".to_owned()); }
        output.flush().map_err(|error| error.to_string())?;
        drop(output);
        if let Some(expected) = &expected_hash {
            notify_hash_check(app, task_id, &file.name);
            if let Err(error) = verify_sha256(&temp, expected) {
                let _ = fs::remove_file(&temp);
                let _ = fs::remove_file(&meta);
                return Err(error);
            }
        }
        fs::rename(&temp, &target).map_err(|error| error.to_string())?;
        let _ = fs::remove_file(&meta);
        progress(app, task_id, &file.name, copied, copied, Some((copied - previous) as f64 / started.elapsed().as_secs_f64().max(0.1)));
        Ok(())
    })();
    // Preserve partial bytes after network errors so Retry can resume with Range.
    if result.is_err() && control.cancelled.load(Ordering::SeqCst) {
        let _ = fs::remove_file(&temp);
        let _ = fs::remove_file(&meta);
    }
    result
}

#[tauri::command]
pub(crate) async fn download_model(app: AppHandle, state: State<'_, Arc<DownloadControl>>, file: ModelFile, model_type: String, task_id: String, resume: bool) -> Result<(), String> {
    if task_id.is_empty() || task_id.len() > 64 || !task_id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-') {
        return Err("Некорректный идентификатор загрузки.".to_owned());
    }
    if state.running.swap(true, Ordering::SeqCst) { return Err("Загрузка уже выполняется.".to_owned()); }
    state.cancelled.store(false, Ordering::SeqCst);
    state.paused.store(false, Ordering::SeqCst);
    let control = Arc::clone(&*state);
    tauri::async_runtime::spawn_blocking(move || {
        let name = file.name.clone();
        let result = download(&app, &control, file, model_type, &task_id, resume);
        let (phase, message) = match result { Ok(()) => ("done", format!("{name} загружен.")), Err(error) => (
            if control.cancelled.load(Ordering::SeqCst) { "cancelled" } else if control.paused.load(Ordering::SeqCst) { "paused" } else { "error" }, error
        ) };
        // Release the worker before notifying the UI so its next queued task can start.
        control.running.store(false, Ordering::SeqCst);
        emit_task(&app, TaskEvent {
            kind: "download", phase, message, percent: None, bytes_per_second: None,
            task_id: Some(task_id), bytes_downloaded: None, total_bytes: None, url: None,
        });
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn pause_download(state: State<'_, Arc<DownloadControl>>) {
    if state.running.load(Ordering::SeqCst) { state.paused.store(true, Ordering::SeqCst); }
}

#[tauri::command]
pub(crate) fn cancel_download(state: State<'_, Arc<DownloadControl>>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub(crate) fn list_downloads(app: AppHandle) -> Result<Vec<DownloadedFile>, String> {
    let library = crate::models_library_dir(&app)?;
    let mut result = Vec::new();
    for (folder, model_type) in [("Stable-diffusion", "Checkpoint"), ("Lora", "LORA"), ("VAE", "VAE")] {
        let root = library.join(folder);
        if !root.exists() { continue; }
        for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry.file_type().map_err(|error| error.to_string())?.is_file() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(base_name) = name.strip_suffix(".flow-meta.json") {
                let saved: PartialMetadata = match serde_json::from_slice(&fs::read(entry.path()).map_err(|error| error.to_string())?) {
                    Ok(saved) => saved,
                    Err(_) => continue,
                };
                if saved.file.name != base_name || safe_name(base_name) != base_name { continue; }
                let target = entry.path().with_file_name(base_name);
                let temp = entry.path().with_file_name(format!("{base_name}.flow-part"));
                if target.exists() { continue; }
                let downloaded_bytes = fs::metadata(temp).map(|metadata| metadata.len()).unwrap_or(0);
                let size_bytes = if saved.file.size_kb.is_finite() && saved.file.size_kb > 0.0 {
                    (saved.file.size_kb * 1024.0) as u64
                } else { 0 };
                result.push(DownloadedFile {
                    name: base_name.to_owned(), model_type: saved.model_type, size_bytes,
                    downloaded_bytes, status: "paused", download_url: Some(saved.file.download_url),
                    sha256: saved.file.sha256,
                });
                continue;
            }
            if name.ends_with(".flow-part") || safe_name(&name) != name
                || !std::path::Path::new(&name).extension().and_then(|value| value.to_str())
                    .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "safetensors" | "ckpt" | "pt" | "pth" | "bin")) { continue; }
            let size_bytes = entry.metadata().map_err(|error| error.to_string())?.len();
            result.push(DownloadedFile {
                name, model_type: model_type.to_owned(), size_bytes, downloaded_bytes: size_bytes,
                status: "completed", download_url: None, sha256: None,
            });
        }
    }
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

#[tauri::command]
pub(crate) fn delete_download(app: AppHandle, model_type: String, file_name: String) -> Result<(), String> {
    if file_name.is_empty() || safe_name(&file_name) != file_name {
        return Err("Некорректное имя файла модели.".to_owned());
    }
    let (target, temp, meta) = paths(&app, &model_type, &file_name)?;
    if target.is_file() { fs::remove_file(&target).map_err(|error| error.to_string())?; }
    if temp.is_file() { fs::remove_file(&temp).map_err(|error| error.to_string())?; }
    if meta.is_file() { fs::remove_file(&meta).map_err(|error| error.to_string())?; }
    // Partial downloads made before category routing lived in Stable-diffusion.
    if model_subfolder(&model_type)? != "Stable-diffusion" {
        let legacy = models_directory(&app)?.join(&file_name);
        let legacy_meta = legacy.with_file_name(format!("{file_name}.flow-meta.json"));
        if legacy_meta.is_file() {
            let saved: PartialMetadata = serde_json::from_slice(&fs::read(&legacy_meta).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            if saved.model_type == model_type && saved.file.name == file_name {
                let legacy_temp = legacy.with_file_name(format!("{file_name}.flow-part"));
                if legacy_temp.is_file() { fs::remove_file(legacy_temp).map_err(|error| error.to_string())?; }
                fs::remove_file(legacy_meta).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{expected_sha256, model_subfolder, verify_sha256};
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn model_types_have_explicit_directories() {
        assert_eq!(model_subfolder("Checkpoint").unwrap(), "Stable-diffusion");
        assert_eq!(model_subfolder("LORA").unwrap(), "Lora");
        assert_eq!(model_subfolder("VAE").unwrap(), "VAE");
        assert!(model_subfolder("Unknown").is_err());
    }

    #[test]
    fn sha256_accepts_only_full_hash_and_detects_damage() {
        let hash = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(expected_sha256(Some(hash)).unwrap().as_deref(), Some(hash));
        assert!(expected_sha256(Some("short")).is_err());
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("flow-sha-{}-{unique}", std::process::id()));
        fs::write(&path, b"hello").unwrap();
        assert!(verify_sha256(&path, hash).is_ok());
        assert!(verify_sha256(&path, &"0".repeat(64)).is_err());
        fs::remove_file(path).unwrap();
    }
}

