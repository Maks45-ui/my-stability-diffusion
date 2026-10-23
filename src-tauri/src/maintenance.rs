use crate::{data_dir, emit_task, installation_dir, installer, TaskEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, sync::Arc};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    fn GetDiskFreeSpaceExW(path: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
}

#[cfg(windows)]
fn windows_resources(parent: &Path) -> Option<(f64, f64, f64, f64)> {
    use std::os::windows::ffi::OsStrExt;
    let mut memory = MemoryStatus { length: std::mem::size_of::<MemoryStatus>() as u32, ..Default::default() };
    let path: Vec<u16> = parent.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut disk_available = 0_u64;
    let memory_ok = unsafe { GlobalMemoryStatusEx(&mut memory) != 0 };
    let disk_ok = unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut disk_available, std::ptr::null_mut(), std::ptr::null_mut()) != 0 };
    if !memory_ok || !disk_ok { return None; }
    let gib = 1024.0 * 1024.0 * 1024.0;
    Some((disk_available as f64 / gib, memory.total_phys as f64 / gib,
        memory.avail_page_file as f64 / gib, memory.total_page_file as f64 / gib))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckEntry {
    label: String,
    value: String,
    level: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckReport {
    title: String,
    checks: Vec<CheckEntry>,
    can_proceed: bool,
}

fn entry(label: &str, value: impl Into<String>, level: &'static str) -> CheckEntry {
    CheckEntry { label: label.to_owned(), value: value.into(), level }
}

fn quiet_output(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    if let Some(cwd) = cwd { command.current_dir(cwd); }
    installer::hide_console(&mut command);
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git(app: &AppHandle, args: &[String]) -> Result<String, String> {
    installer::command_output(app, "git", args)
}

fn environment_checks(parent: &Path, package_id: &str) -> Vec<CheckEntry> {
    let mut checks = Vec::new();
    match quiet_output("git", &["--version"], None) {
        Ok(version) => checks.push(entry("Git", version, "ok")),
        Err(_) => checks.push(entry("Git", "Не найден — установка невозможна", "error")),
    }
    if package_id == "automatic1111" {
        let python = quiet_output("py", &["-3.10", "-c", "import sys; print(sys.version.split()[0])"], None)
            .or_else(|_| quiet_output("python", &["-c", "import sys; print(sys.version.split()[0])"], None));
        match python {
            Ok(version) if version.starts_with("3.10.") => checks.push(entry("Python", version, "ok")),
            _ => checks.push(entry("Python", "Требуется Python 3.10", "error")),
        }
    } else {
        let uv = quiet_output("uv", &["--version"], None).is_ok();
        let bootstrap = quiet_output("py", &["-3.10", "-c", "import sys; print(sys.version.split()[0])"], None)
            .or_else(|_| quiet_output("python", &["-c", "import sys; print(sys.version.split()[0])"], None))
            .is_ok_and(|version| version.starts_with("3.10."));
        checks.push(entry("Python/uv", if uv { "uv подготовит Python 3.13" } else if bootstrap { "Python 3.10 установит uv и Python 3.13" } else { "Нужен uv или Python 3.10 для подготовки Python 3.13" }, if uv || bootstrap { "ok" } else { "error" }));
    }
    #[cfg(windows)] {
        if let Some((disk, ram, free_commit, limit)) = windows_resources(parent) {
            checks.push(entry("Свободно на диске", format!("{disk:.1} ГБ"), if disk < 5.0 { "error" } else if disk < 20.0 { "warning" } else { "ok" }));
            checks.push(entry("Оперативная память", format!("{ram:.1} ГБ"), if ram < 8.0 { "warning" } else { "ok" }));
            checks.push(entry("Доступная виртуальная память", format!("{free_commit:.1} / {limit:.1} ГБ"), if free_commit < 10.0 { "warning" } else { "ok" }));
            let pagefile = (limit - ram).max(0.0);
            checks.push(entry("Файл подкачки", format!("≈ {pagefile:.1} ГБ выделено Windows"), if pagefile < 16.0 { "warning" } else { "ok" }));
        } else { checks.push(entry("Память и диск", "Не удалось прочитать системные показатели", "warning")); }
    }
    let gpu = quiet_output("nvidia-smi", &["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"], None);
    match gpu {
        Ok(line) => {
            let parts: Vec<_> = line.lines().next().unwrap_or("").split(',').map(str::trim).collect();
            let mib = parts.get(1).and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
            checks.push(entry("Видеокарта", format!("{} · {} МиБ VRAM · драйвер {}", parts.first().unwrap_or(&"NVIDIA"), mib, parts.get(2).unwrap_or(&"?")), if mib > 0 && mib < 6144 { "warning" } else { "ok" }));
        }
        Err(_) => checks.push(entry("Видеокарта", "NVIDIA не обнаружена; режим CUDA не проверен", "warning")),
    }
    checks
}

#[tauri::command]
pub(crate) async fn get_preflight(app: AppHandle, package_id: String) -> Result<CheckReport, String> {
    if package_id != "automatic1111" && package_id != "forge-neo" { return Err("Неизвестный пакет.".to_owned()); }
    tauri::async_runtime::spawn_blocking(move || {
        let parent = installation_dir(&app)?;
        let checks = environment_checks(&parent, &package_id);
        let can_proceed = !checks.iter().any(|item| item.level == "error");
        Ok(CheckReport { title: "Проверка перед установкой".to_owned(), checks, can_proceed })
    }).await.map_err(|error| error.to_string())?
}

fn managed_package(app: &AppHandle, package_id: &str) -> Result<(PathBuf, PathBuf), String> {
    let parent = installation_dir(app)?;
    let directory = match package_id {
        "automatic1111" => parent.join("sd-webui"),
        "forge-neo" => parent.join("sd-webui-forge-neo"),
        _ => return Err("Неизвестный пакет.".to_owned()),
    };
    if !directory.join(".flow-installed.json").is_file() {
        return Err("Обслуживание доступно только для пакета, установленного Flow. Подключённый пакет Stability Matrix не изменяется.".to_owned());
    }
    Ok((parent, directory))
}

#[tauri::command]
pub(crate) async fn check_package(app: AppHandle, package_id: String) -> Result<CheckReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (_, directory) = managed_package(&app, &package_id)?;
        let mut checks = environment_checks(&directory, &package_id);
        checks.push(entry("Файлы WebUI", if directory.join("launch.py").is_file() { "launch.py найден" } else { "launch.py отсутствует" }, if directory.join("launch.py").is_file() { "ok" } else { "error" }));
        if package_id == "forge-neo" {
            checks.push(entry("Forge", if directory.join("modules_forge").is_dir() { "Модули найдены" } else { "Модули отсутствуют" }, if directory.join("modules_forge").is_dir() { "ok" } else { "error" }));
        }
        let python = installer::venv_python(&directory);
        if python.is_file() {
            let probe = "import sys, json, importlib.metadata as m; d={x.metadata.get('Name','').lower():x.version for x in m.distributions()}; print(json.dumps({'python':sys.version.split()[0], 'torch':d.get('torch','нет'), 'safetensors':d.get('safetensors','нет'), 'gradio':d.get('gradio','нет')}))";
            match quiet_output(&python.to_string_lossy(), &["-c", probe], Some(&directory)) {
                Ok(result) => {
                    if let Ok(versions) = serde_json::from_str::<Value>(&result) {
                        for (label, key) in [("Python venv", "python"), ("PyTorch", "torch"), ("safetensors", "safetensors"), ("Gradio", "gradio")] {
                            let version = versions[key].as_str().unwrap_or("нет");
                            checks.push(entry(label, version, if version == "нет" { "error" } else { "ok" }));
                        }
                    } else { checks.push(entry("Окружение", "Не удалось прочитать версии", "error")); }
                }
                Err(error) => checks.push(entry("Окружение", error, "error")),
            }
        } else { checks.push(entry("Окружение", "Python venv отсутствует", "error")); }
        let git_dir = directory.to_string_lossy().into_owned();
        let origin = quiet_output("git", &["-C", &git_dir, "remote", "get-url", "origin"], None).unwrap_or_default();
        let expected = if package_id == "forge-neo" { "Haoming02/sd-webui-forge-classic" } else { "AUTOMATIC1111/stable-diffusion-webui" };
        checks.push(entry("Исходный репозиторий", if origin.is_empty() { "Не найден" } else { &origin }, if origin.contains(expected) { "ok" } else { "error" }));
        if let Ok(dir) = data_dir(&app) {
            let logs = dir.join("logs");
            if let Ok(entries) = fs::read_dir(logs) {
                if let Some(latest) = entries.flatten().filter(|item| item.file_name().to_string_lossy().ends_with("stderr.log")).max_by_key(|item| item.metadata().ok().and_then(|meta| meta.modified().ok())) {
                    checks.push(entry("Последний журнал ошибок", latest.path().display().to_string(), "info"));
                }
            }
        }
        let can_proceed = !checks.iter().any(|item| item.level == "error");
        Ok(CheckReport { title: format!("Проверка {}", if package_id == "forge-neo" { "Forge Neo" } else { "AUTOMATIC1111" }), checks, can_proceed })
    }).await.map_err(|error| error.to_string())?
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RollbackSnapshot { package_id: String, revision: String }

fn rollback_file(app: &AppHandle, package_id: &str) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(format!("rollback-{package_id}.json")))
}

pub(crate) fn has_rollback(app: &AppHandle, package_id: &str) -> bool {
    rollback_file(app, package_id).is_ok_and(|path| path.is_file())
}

fn safe_revision(revision: &str) -> bool {
    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn run_maintenance(app: &AppHandle, control: &installer::InstallControl, package_id: &str, action: &str) -> Result<(), String> {
    let (parent, directory) = managed_package(app, package_id)?;
    let expected = if package_id == "forge-neo" { "https://github.com/Haoming02/sd-webui-forge-classic" } else { "https://github.com/AUTOMATIC1111/stable-diffusion-webui" };
    let dir = directory.to_string_lossy().into_owned();
    let remote = git(app, &["-C".into(), dir.clone(), "remote".into(), "get-url".into(), "origin".into()])?;
    if remote.trim_end_matches(".git").trim_end_matches('/') != expected {
        return Err("Адрес Git-репозитория изменён. Обновление/ремонт остановлены для защиты пакета.".to_owned());
    }
    if action != "repair" {
        let dirty = git(app, &["-C".into(), dir.clone(), "status".into(), "--porcelain".into(), "--untracked-files=no".into()])?;
        if !dirty.is_empty() { return Err("В исходниках WebUI есть локальные изменения. Сохраните их перед обновлением или откатом.".to_owned()); }
    }
    if action == "update" {
        let previous = git(app, &["-C".into(), dir.clone(), "rev-parse".into(), "HEAD".into()])?;
        if !safe_revision(&previous) { return Err("Не удалось определить текущую ревизию пакета.".to_owned()); }
        let branch = if package_id == "forge-neo" { "neo" } else { "HEAD" };
        installer::run_logged(app, control, "git", &["fetch".into(), "--progress".into(), "origin".into(), branch.into()], &directory, None)?;
        let newest = git(app, &["-C".into(), dir.clone(), "rev-parse".into(), "FETCH_HEAD".into()])?;
        if newest == previous {
            emit_task(app, TaskEvent { kind: "install", phase: "done", message: "Пакет уже обновлён до последней доступной ревизии.".to_owned(), url: None, percent: None, bytes_per_second: None, task_id: None, bytes_downloaded: None, total_bytes: None });
            return Ok(());
        }
        // Keep the old object reachable even if the managed checkout is shallow.
        installer::run_logged(app, control, "git", &["update-ref".into(), "refs/flow/rollback".into(), previous.clone()], &directory, None)?;
        let snapshot = RollbackSnapshot { package_id: package_id.to_owned(), revision: previous.clone() };
        let file = rollback_file(app, package_id)?;
        fs::create_dir_all(file.parent().ok_or("Нет папки состояния обновлений")?).map_err(|error| error.to_string())?;
        fs::write(&file, serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        installer::run_logged(app, control, "git", &["reset".into(), "--hard".into(), newest], &directory, None)?;
    } else if action == "rollback" {
        let snapshot: RollbackSnapshot = serde_json::from_slice(&fs::read(rollback_file(app, package_id)?).map_err(|_| "Нет сохранённой версии для отката.")?).map_err(|error| error.to_string())?;
        if snapshot.package_id != package_id || !safe_revision(&snapshot.revision) { return Err("Сохранённая ревизия отката повреждена.".to_owned()); }
        installer::run_logged(app, control, "git", &["reset".into(), "--hard".into(), snapshot.revision], &directory, None)?;
    }
    if package_id == "forge-neo" { installer::install_forge_neo(app, control, &parent) }
    else { installer::install(app, control, &parent) }
}

fn start_maintenance(app: AppHandle, control: Arc<installer::InstallControl>, package_id: String, action: &'static str) -> Result<(), String> {
    control.try_start()?;
    tauri::async_runtime::spawn_blocking(move || {
        emit_task(&app, TaskEvent { kind: "install", phase: "checking", message: format!("{}: {}", package_id, action), url: None, percent: None, bytes_per_second: None, task_id: None, bytes_downloaded: None, total_bytes: None });
        let result = run_maintenance(&app, &control, &package_id, action);
        if let Err(error) = result {
            emit_task(&app, TaskEvent { kind: "install", phase: if control.was_cancelled() { "cancelled" } else { "error" }, message: error, url: None, percent: None, bytes_per_second: None, task_id: None, bytes_downloaded: None, total_bytes: None });
        }
        control.finish();
    });
    Ok(())
}

fn maintenance_guard(webui: &installer::WebuiControl, downloads: &crate::civitai::DownloadControl) -> Result<(), String> {
    if webui.is_running() || downloads.is_running() { Err("Остановите WebUI и загрузки перед обслуживанием пакета.".to_owned()) } else { Ok(()) }
}

#[tauri::command]
pub(crate) fn repair_package(app: AppHandle, package_id: String, installs: State<'_, Arc<installer::InstallControl>>, webui: State<'_, installer::WebuiControl>, downloads: State<'_, Arc<crate::civitai::DownloadControl>>) -> Result<(), String> {
    maintenance_guard(&webui, &downloads)?;
    managed_package(&app, &package_id)?;
    start_maintenance(app, Arc::clone(&installs), package_id, "repair")
}

#[tauri::command]
pub(crate) fn update_package(app: AppHandle, package_id: String, installs: State<'_, Arc<installer::InstallControl>>, webui: State<'_, installer::WebuiControl>, downloads: State<'_, Arc<crate::civitai::DownloadControl>>) -> Result<(), String> {
    maintenance_guard(&webui, &downloads)?;
    managed_package(&app, &package_id)?;
    start_maintenance(app, Arc::clone(&installs), package_id, "update")
}

#[tauri::command]
pub(crate) fn rollback_package(app: AppHandle, package_id: String, installs: State<'_, Arc<installer::InstallControl>>, webui: State<'_, installer::WebuiControl>, downloads: State<'_, Arc<crate::civitai::DownloadControl>>) -> Result<(), String> {
    maintenance_guard(&webui, &downloads)?;
    managed_package(&app, &package_id)?;
    if !rollback_file(&app, &package_id)?.is_file() { return Err("Сохранённой версии для отката пока нет.".to_owned()); }
    start_maintenance(app, Arc::clone(&installs), package_id, "rollback")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlowUpdate { current_version: String, latest_version: Option<String>, available: bool, url: String }

#[tauri::command]
pub(crate) async fn check_flow_update() -> Result<FlowUpdate, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let current = env!("CARGO_PKG_VERSION").to_owned();
        let url = "https://github.com/Maks45-ui/my-stability-diffusion/releases".to_owned();
        let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(8)).build().map_err(|error| error.to_string())?;
        let response = client.get("https://api.github.com/repos/Maks45-ui/my-stability-diffusion/releases/latest")
            .header("User-Agent", "Flow-Updater").send().map_err(|error| error.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND { return Ok(FlowUpdate { current_version: current, latest_version: None, available: false, url }); }
        let release: Value = response.error_for_status().map_err(|error| error.to_string())?.json().map_err(|error| error.to_string())?;
        let latest = release["tag_name"].as_str().unwrap_or("").trim_start_matches('v');
        let version = |text: &str| -> Vec<u64> { text.split('.').map(|part| part.parse().unwrap_or(0)).collect() };
        let release_url = release["html_url"].as_str().filter(|value| value.starts_with("https://github.com/Maks45-ui/my-stability-diffusion/releases/tag/"))
            .unwrap_or(&url).to_owned();
        Ok(FlowUpdate { current_version: current.clone(), latest_version: if latest.is_empty() { None } else { Some(latest.to_owned()) }, available: !latest.is_empty() && version(latest) > version(&current), url: release_url })
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn open_flow_releases(app: AppHandle) -> Result<(), String> {
    app.opener().open_url("https://github.com/Maks45-ui/my-stability-diffusion/releases", None::<&str>).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn link_package_models(app: AppHandle, package_id: String, downloads: State<'_, Arc<crate::civitai::DownloadControl>>) -> Result<String, String> {
    if downloads.is_running() { return Err("Дождитесь окончания загрузки модели.".to_owned()); }
    if crate::active_package(&app)? != package_id { return Err("Сначала выберите этот пакет.".to_owned()); }
    tauri::async_runtime::spawn_blocking(move || {
        let source = crate::package_dir(&app)?.join("models");
        let destination = crate::models_library_dir(&app)?;
        if !source.is_dir() { return Err("В пакете ещё нет папки с чекпоинтами.".to_owned()); }
        let (linked, conflicts) = link_model_tree(&source, &destination)?;
        Ok(format!("Связано без копирования: {linked}; совпадающие имена: {conflicts}. Для жёстких ссылок папки должны быть на одном диске NTFS."))
    }).await.map_err(|error| error.to_string())?
}

fn link_contents(source: &Path, destination: &Path, depth: usize, linked: &mut usize, conflicts: &mut usize) -> Result<(), String> {
    if depth > 12 { return Err("Слишком глубокая структура папок моделей.".to_owned()); }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for item in fs::read_dir(source).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        let kind = item.file_type().map_err(|error| error.to_string())?;
        if kind.is_symlink() { continue; }
        let target = destination.join(item.file_name());
        if kind.is_dir() {
            link_contents(&item.path(), &target, depth + 1, linked, conflicts)?;
        } else if kind.is_file() {
            let name = item.file_name().to_string_lossy().to_ascii_lowercase();
            if name.ends_with(".flow-part") || name.ends_with(".flow-meta.json") { continue; }
            if target.exists() { *conflicts += 1; continue; }
            fs::hard_link(item.path(), target).map_err(|error| format!("Не удалось связать {}: {error}. Папки моделей должны быть на одном диске NTFS.", item.path().display()))?;
            *linked += 1;
        }
    }
    Ok(())
}

pub(crate) fn link_model_tree(source: &Path, destination: &Path) -> Result<(usize, usize), String> {
    if !source.is_dir() { return Ok((0, 0)); }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let destination = destination.canonicalize().map_err(|error| error.to_string())?;
    if source == destination { return Ok((0, 0)); }
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err("Общая библиотека должна находиться отдельно от папки моделей пакета.".to_owned());
    }
    let mut linked = 0;
    let mut conflicts = 0;
    link_contents(&source, &destination, 0, &mut linked, &mut conflicts)?;
    Ok((linked, conflicts))
}
