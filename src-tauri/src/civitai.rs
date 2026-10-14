use crate::{civitai_token, emit_task, installation_dir, TaskEvent};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelFile {
    name: String,
    download_url: String,
    size_kb: f64,
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
    images: Vec<String>,
    files: Vec<ModelFile>,
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
pub(crate) async fn search_models(app: AppHandle, query: String) -> Result<Vec<ModelSummary>, String> {
    let trimmed = query.trim();
    let mut request = client()?.get(API).query(&[("limit", "12"), ("nsfw", "true")]);
    if !trimmed.is_empty() {
        request = if trimmed.parse::<u64>().is_ok() { request.query(&[("ids", trimmed)]) } else { request.query(&[("query", trimmed)]) };
    } else { request = request.query(&[("sort", "Most Downloaded")]); }
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
    Ok(installation_dir(app)?.join("sd-webui").join("models").join("Stable-diffusion"))
}

fn paths(app: &AppHandle, file_name: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let name = safe_name(file_name);
    if name.is_empty() || name != file_name { return Err("Некорректное имя файла модели.".to_owned()); }
    let target = models_directory(app)?.join(&name);
    let temp = target.with_file_name(format!("{name}.flow-part"));
    let meta = target.with_file_name(format!("{name}.flow-meta.json"));
    Ok((target, temp, meta))
}

fn progress(app: &AppHandle, task_id: &str, file_name: &str, copied: u64, total: u64, speed: Option<f64>) {
    emit_task(app, TaskEvent {
        kind: "download", phase: "dependencies", message: file_name.to_owned(),
        percent: if total > 0 { Some((copied.saturating_mul(100) / total).min(100) as u8) } else { None },
        bytes_per_second: speed, task_id: Some(task_id.to_owned()),
        bytes_downloaded: Some(copied), total_bytes: Some(total), url: None,
    });
}

fn download(app: &AppHandle, control: &DownloadControl, file: ModelFile, model_type: String, task_id: &str, resume: bool) -> Result<(), String> {
    let mut url = allowed_url(&file.download_url)?;
    if !url.path().starts_with("/api/download/models/") { return Err("Недопустимая ссылка скачивания.".to_owned()); }
    url.set_host(Some("civitai.red")).map_err(|_| "Недопустимый адрес скачивания.".to_owned())?;
    if !installation_dir(app)?.join("sd-webui").join(".flow-installed.json").is_file() {
        return Err("Сначала установите пакет WebUI, чтобы загружать модели в его папку.".to_owned());
    }
    let (target, temp, meta) = paths(app, &file.name)?;
    if let Some(folder) = target.parent() { fs::create_dir_all(folder).map_err(|error| error.to_string())?; }
    if target.exists() { return Err("Файл уже загружен.".to_owned()); }
    if !resume && temp.exists() { return Err("Найдена незавершённая загрузка. Удалите её перед повтором.".to_owned()); }
    if resume && temp.exists() && !meta.exists() { return Err("Не найдены сведения для продолжения загрузки.".to_owned()); }
    if resume && meta.exists() {
        let saved: PartialMetadata = serde_json::from_slice(&fs::read(&meta).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if saved.file.name != file.name || saved.file.download_url != file.download_url || saved.model_type != model_type {
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
        fs::rename(&temp, &target).map_err(|error| error.to_string())?;
        let _ = fs::remove_file(&meta);
        progress(app, task_id, &file.name, copied, copied, Some((copied - previous) as f64 / started.elapsed().as_secs_f64().max(0.1)));
        Ok(())
    })();
    if result.is_err() && !control.paused.load(Ordering::SeqCst) {
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
    let root = models_directory(&app)?;
    if !root.exists() { return Ok(Vec::new()); }
    let mut result = Vec::new();
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
            let (target, temp, _) = paths(&app, base_name)?;
            if target.exists() { continue; }
            let downloaded_bytes = fs::metadata(temp).map(|metadata| metadata.len()).unwrap_or(0);
            let size_bytes = if saved.file.size_kb.is_finite() && saved.file.size_kb > 0.0 {
                (saved.file.size_kb * 1024.0) as u64
            } else { 0 };
            result.push(DownloadedFile {
                name: base_name.to_owned(), model_type: saved.model_type, size_bytes,
                downloaded_bytes, status: "paused", download_url: Some(saved.file.download_url),
            });
            continue;
        }
        if name.ends_with(".flow-part") || safe_name(&name) != name { continue; }
        let size_bytes = entry.metadata().map_err(|error| error.to_string())?.len();
        result.push(DownloadedFile {
            name, model_type: "Stable-diffusion".to_owned(), size_bytes, downloaded_bytes: size_bytes,
            status: "completed", download_url: None,
        });
    }
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

#[tauri::command]
pub(crate) fn delete_download(app: AppHandle, _model_type: String, file_name: String) -> Result<(), String> {
    if safe_name(&file_name) != file_name {
        return Err("Некорректное имя файла модели.".to_owned());
    }
    let (target, temp, meta) = paths(&app, &file_name)?;
    // These paths are constructed below the installed package's model directory.
    if target.is_file() { fs::remove_file(target).map_err(|error| error.to_string())?; }
    if temp.is_file() { fs::remove_file(temp).map_err(|error| error.to_string())?; }
    if meta.is_file() { fs::remove_file(meta).map_err(|error| error.to_string())?; }
    Ok(())
}
