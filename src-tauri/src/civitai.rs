use crate::{civitai_token, data_dir, emit_task, TaskEvent};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs::{self, File}, io::{Read, Write}, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::Instant};
use tauri::{AppHandle, State};
use url::Url;

const API: &str = "https://civitai.red/api/v1/models";

#[derive(Default)]
pub(crate) struct DownloadControl {
    running: AtomicBool,
    cancelled: AtomicBool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelFile {
    name: String,
    download_url: String,
    size_kb: f64,
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
                            files.push(ModelFile {
                                name: name.to_owned(), download_url: download_url.to_owned(),
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
    value.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ')).take(150).collect::<String>().trim_matches(|c| c == '.' || c == ' ').to_owned()
}

fn download(app: &AppHandle, control: &DownloadControl, file: ModelFile, model_type: String) -> Result<(), String> {
    let mut url = allowed_url(&file.download_url)?;
    if !url.path().starts_with("/api/download/models/") { return Err("Недопустимая ссылка скачивания.".to_owned()); }
    url.set_host(Some("civitai.red")).map_err(|_| "Недопустимый адрес скачивания.".to_owned())?;
    let name = safe_name(&file.name);
    if name.is_empty() { return Err("Некорректное имя файла.".to_owned()); }
    let folder = data_dir(app)?.join("models").join(safe_name(&model_type));
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let target = folder.join(name);
    let temp: PathBuf = target.with_extension("flow-part");
    if target.exists() || temp.exists() { return Err("Файл уже существует или загрузка не завершена.".to_owned()); }
    let token = civitai_token(app)?;
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(300)).build().map_err(|error| error.to_string())?;
    let mut request = client.get(url);
    if !token.is_empty() { request = request.bearer_auth(token); }
    let mut response = request.send().map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?;
    let total = response.content_length().unwrap_or(0);
    let mut output = File::create(&temp).map_err(|error| error.to_string())?;
    let started = Instant::now();
    let result = (|| -> Result<(), String> {
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if control.cancelled.load(Ordering::SeqCst) { return Err("Загрузка отменена.".to_owned()); }
            let count = response.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 { break; }
            output.write_all(&buffer[..count]).map_err(|error| error.to_string())?;
            copied += count as u64;
            emit_task(app, TaskEvent {
                kind: "download", phase: "dependencies", message: file.name.clone(),
                percent: if total > 0 { Some((copied.saturating_mul(100) / total).min(100) as u8) } else { None },
                bytes_per_second: Some(copied as f64 / started.elapsed().as_secs_f64().max(0.1)),
            });
        }
        output.flush().map_err(|error| error.to_string())?;
        drop(output);
        fs::rename(&temp, &target).map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result
}

#[tauri::command]
pub(crate) async fn download_model(app: AppHandle, state: State<'_, Arc<DownloadControl>>, file: ModelFile, model_type: String) -> Result<(), String> {
    if state.running.swap(true, Ordering::SeqCst) { return Err("Загрузка уже выполняется.".to_owned()); }
    state.cancelled.store(false, Ordering::SeqCst);
    let control = Arc::clone(&*state);
    tauri::async_runtime::spawn_blocking(move || {
        let name = file.name.clone();
        let result = download(&app, &control, file, model_type);
        let (phase, message) = match result { Ok(()) => ("done", format!("{name} загружен.")), Err(error) => (if control.cancelled.load(Ordering::SeqCst) { "cancelled" } else { "error" }, error) };
        emit_task(&app, TaskEvent { kind: "download", phase, message, percent: None, bytes_per_second: None });
        control.running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_download(state: State<'_, Arc<DownloadControl>>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

