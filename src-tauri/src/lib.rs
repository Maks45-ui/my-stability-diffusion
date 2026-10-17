mod civitai;
mod discord;
mod gallery;
mod installer;
mod paths;

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    civitai_token: String,
    #[serde(default)]
    installation_dir: Option<String>,
    #[serde(default)]
    webui_arguments: Vec<String>,
    #[serde(default)]
    sdxl_memory_profile: bool,
    #[serde(default)]
    onboarding_complete: bool,
    #[serde(default)]
    onboarding_stage: u8,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebuiSettings {
    arguments: Vec<String>,
    sdxl_memory_profile: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppState {
    packages: Vec<PackageInfo>,
    has_civitai_token: bool,
    models_path: String,
    installation_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageInfo {
    name: String,
    version: String,
    path: String,
    running: bool,
    stopping: bool,
    stopped: bool,
    ready_url: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskEvent {
    pub kind: &'static str,
    pub phase: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_downloaded: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
}

pub(crate) fn emit_task(app: &AppHandle, event: TaskEvent) {
    let _ = app.emit("flow:task", event);
}

pub(crate) fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(|error| error.to_string())
}

fn settings_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("settings.json"))
}

fn read_settings(app: &AppHandle) -> Result<Settings, String> {
    let path = settings_file(app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_file(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

pub(crate) fn installation_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(saved) = read_settings(app)?.installation_dir {
        return Ok(paths::subprocess_path(&PathBuf::from(saved)));
    }
    let default = data_dir(app)?.join("packages");
    fs::create_dir_all(&default).map_err(|error| error.to_string())?;
    Ok(paths::subprocess_path(&default))
}

pub(crate) fn webui_arguments(app: &AppHandle) -> Result<Vec<String>, String> {
    Ok(read_settings(app)?.webui_arguments)
}

pub(crate) fn sdxl_memory_profile(app: &AppHandle) -> Result<bool, String> {
    Ok(read_settings(app)?.sdxl_memory_profile)
}

pub(crate) fn civitai_token(app: &AppHandle) -> Result<String, String> {
    let saved = read_settings(app)?.civitai_token;
    if !saved.is_empty() { return Ok(saved); }
    Ok(std::env::var("CIVITAI_API_KEY").unwrap_or_default())
}

#[tauri::command]
fn get_state(app: AppHandle, webui: State<'_, installer::WebuiControl>) -> Result<AppState, String> {
    let parent = installation_dir(&app)?;
    let package = parent.join("sd-webui");
    let marker = package.join(".flow-installed.json");
    let packages = if marker.is_file() {
        let version = fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("version").and_then(serde_json::Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "Установлен".to_owned());
        vec![PackageInfo {
            name: "AUTOMATIC1111 WebUI".to_owned(),
            version,
            path: package.to_string_lossy().into_owned(),
            running: webui.is_running(),
            stopping: webui.is_stopping(),
            stopped: webui.was_stopped(),
            ready_url: webui.ready_url(),
        }]
    } else {
        Vec::new()
    };
    let token = civitai_token(&app)?;
    Ok(AppState {
        packages,
        has_civitai_token: !token.is_empty(),
        models_path: civitai::models_directory(&app)?.to_string_lossy().into_owned(),
        installation_dir: parent.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn set_installation_dir(
    app: AppHandle,
    directory: String,
    installs: State<'_, Arc<installer::InstallControl>>,
    webui: State<'_, installer::WebuiControl>,
    downloads: State<'_, Arc<civitai::DownloadControl>>,
) -> Result<(), String> {
    if installs.is_running() || webui.is_running() || downloads.is_running() {
        return Err("Дождитесь завершения установки, работы WebUI или загрузки модели перед сменой папки.".to_owned());
    }
    let path = paths::subprocess_path(&PathBuf::from(directory.trim()));
    // The dialog returns an existing directory. Recheck in Rust before persisting it.
    if !path.is_absolute() || !path.is_dir() {
        return Err("Выберите существующую абсолютную папку установки.".to_owned());
    }
    let normalized = paths::subprocess_path(&path.canonicalize().map_err(|error| error.to_string())?);
    let mut settings = read_settings(&app)?;
    settings.installation_dir = Some(normalized.to_string_lossy().into_owned());
    write_settings(&app, &settings)
}

#[tauri::command]
fn save_civitai_token(app: AppHandle, token: String) -> Result<(), String> {
    if token.len() > 512 {
        return Err("Некорректный API-ключ.".to_owned());
    }
    let mut settings = read_settings(&app)?;
    settings.civitai_token = token.trim().to_owned();
    write_settings(&app, &settings)
}

#[tauri::command]
fn get_onboarding_complete(app: AppHandle) -> Result<bool, String> {
    let mut settings = read_settings(&app)?;
    // Existing installations with a saved key have already configured Civitai.
    if !settings.onboarding_complete && settings.onboarding_stage == 0 && !settings.civitai_token.is_empty() {
        settings.onboarding_complete = true;
        write_settings(&app, &settings)?;
    }
    Ok(settings.onboarding_complete)
}

#[tauri::command]
fn get_onboarding_stage(app: AppHandle) -> Result<u8, String> {
    if get_onboarding_complete(app.clone())? { return Ok(2); }
    Ok(read_settings(&app)?.onboarding_stage.min(1))
}

#[tauri::command]
fn save_onboarding_token(app: AppHandle, token: Option<String>) -> Result<(), String> {
    let mut settings = read_settings(&app)?;
    apply_onboarding_token(&mut settings, token)?;
    write_settings(&app, &settings)
}

fn apply_onboarding_token(settings: &mut Settings, token: Option<String>) -> Result<(), String> {
    if let Some(token) = token {
        let token = token.trim();
        if token.is_empty() || token.len() > 512 || token.chars().any(char::is_whitespace) {
            return Err("Введите корректный API-ключ без пробелов.".to_owned());
        }
        settings.civitai_token = token.to_owned();
    }
    settings.onboarding_stage = 1;
    Ok(())
}

#[tauri::command]
fn finish_onboarding(app: AppHandle) -> Result<(), String> {
    let mut settings = read_settings(&app)?;
    if settings.onboarding_stage != 1 { return Err("Сначала завершите шаг с API-ключом.".to_owned()); }
    settings.onboarding_complete = true;
    settings.onboarding_stage = 2;
    write_settings(&app, &settings)
}

#[tauri::command]
fn complete_onboarding(app: AppHandle, token: Option<String>) -> Result<(), String> {
    let mut settings = read_settings(&app)?;
    apply_onboarding_choice(&mut settings, token)?;
    // The key and completion flag are persisted together; the UI closes only on success.
    write_settings(&app, &settings)
}

fn apply_onboarding_choice(settings: &mut Settings, token: Option<String>) -> Result<(), String> {
    apply_onboarding_token(settings, token)?;
    settings.onboarding_complete = true;
    settings.onboarding_stage = 2;
    Ok(())
}

#[tauri::command]
fn open_civitai_site(app: AppHandle) -> Result<(), String> {
    app.opener().open_url("https://civitai.red/", None::<&str>).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_webui_arguments(app: AppHandle) -> Result<Vec<String>, String> {
    webui_arguments(&app)
}

#[tauri::command]
fn save_webui_arguments(app: AppHandle, arguments: Vec<String>) -> Result<Vec<String>, String> {
    installer::parse_webui_arguments(&arguments)?;
    let normalized: Vec<String> = arguments.into_iter().map(|argument| argument.trim().to_owned()).collect();
    let mut settings = read_settings(&app)?;
    settings.webui_arguments = normalized.clone();
    write_settings(&app, &settings)?;
    Ok(normalized)
}

#[tauri::command]
fn get_webui_settings(app: AppHandle) -> Result<WebuiSettings, String> {
    let settings = read_settings(&app)?;
    Ok(WebuiSettings { arguments: settings.webui_arguments, sdxl_memory_profile: settings.sdxl_memory_profile })
}

#[tauri::command]
fn save_webui_settings(app: AppHandle, settings: WebuiSettings) -> Result<WebuiSettings, String> {
    installer::parse_webui_arguments(&settings.arguments)?;
    let mut saved = read_settings(&app)?;
    saved.webui_arguments = settings.arguments.into_iter().map(|argument| argument.trim().to_owned()).collect();
    saved.sdxl_memory_profile = settings.sdxl_memory_profile;
    write_settings(&app, &saved)?;
    Ok(WebuiSettings { arguments: saved.webui_arguments, sdxl_memory_profile: saved.sdxl_memory_profile })
}

#[tauri::command]
fn set_page(page: String) {
    discord::set_page(&page);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    configure_webview2();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(installer::InstallControl::default()))
        .manage(installer::WebuiControl::default())
        .manage(Arc::new(civitai::DownloadControl::default()))
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_installation_dir,
            save_civitai_token,
            get_onboarding_complete,
            get_onboarding_stage,
            save_onboarding_token,
            finish_onboarding,
            complete_onboarding,
            open_civitai_site,
            get_webui_arguments,
            save_webui_arguments,
            get_webui_settings,
            save_webui_settings,
            gallery::list_gallery_images,
            set_page,
            installer::install_web_ui,
            installer::cancel_install,
            installer::launch_web_ui,
            installer::stop_web_ui,
            installer::open_web_ui,
            installer::check_xformers,
            installer::install_xformers,
            civitai::search_models,
            civitai::get_starter_model,
            civitai::download_model,
            civitai::pause_download,
            civitai::cancel_download,
            civitai::list_downloads,
            civitai::delete_download,
            civitai::get_preview
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Flow");
}

#[cfg(windows)]
fn configure_webview2() {
    // Diagnostic fallback only. WebUI itself opens in the system browser, outside this WebView2.
    let requested = std::env::var("FLOW_WEBVIEW2_SOFTWARE_RENDERING").as_deref() == Ok("1")
        || std::env::args_os().any(|argument| argument.to_string_lossy() == "--flow-software-rendering");
    if !requested { return; }
    let current = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    if current.split_whitespace().any(|argument| argument == "--disable-gpu") { return; }
    let flags = if current.trim().is_empty() { "--disable-gpu".to_owned() }
        else { format!("{} --disable-gpu", current.trim()) };
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", flags);
}

#[cfg(test)]
mod onboarding_tests {
    use super::{apply_onboarding_choice, apply_onboarding_token, Settings};

    #[test]
    fn api_key_step_does_not_finish_onboarding() {
        let mut settings = Settings::default();
        apply_onboarding_token(&mut settings, Some(" sample-key ".to_owned())).unwrap();
        assert_eq!(settings.onboarding_stage, 1);
        assert!(!settings.onboarding_complete);
        assert_eq!(settings.civitai_token, "sample-key");
    }

    #[test]
    fn skip_and_save_complete_onboarding() {
        let mut skipped = Settings::default();
        apply_onboarding_choice(&mut skipped, None).unwrap();
        assert!(skipped.onboarding_complete);
        assert!(skipped.civitai_token.is_empty());

        let mut saved = Settings::default();
        apply_onboarding_choice(&mut saved, Some("  sample-key  ".to_owned())).unwrap();
        assert!(saved.onboarding_complete);
        assert_eq!(saved.civitai_token, "sample-key");
    }

    #[test]
    fn invalid_key_does_not_complete_onboarding() {
        let mut settings = Settings::default();
        assert!(apply_onboarding_choice(&mut settings, Some("  ".to_owned())).is_err());
        assert!(!settings.onboarding_complete);
    }
}
