use crate::{emit_task, installation_dir, TaskEvent};
use serde_json::json;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, State};

const WEBUI_REPO: &str = "https://github.com/AUTOMATIC1111/stable-diffusion-webui.git";
const SD_REPO: &str = "https://github.com/w-e-w/stablediffusion.git";

#[derive(Default)]
pub(crate) struct InstallControl {
    running: AtomicBool,
    cancelled: AtomicBool,
    pid: Mutex<Option<u32>>,
}

#[derive(Default)]
pub(crate) struct WebuiControl(Mutex<Option<Child>>);

impl WebuiControl {
    pub(crate) fn is_running(&self) -> bool {
        let Ok(mut slot) = self.0.lock() else { return false };
        let Some(child) = slot.as_mut() else { return false };
        match child.try_wait() {
            Ok(None) => true,
            _ => { *slot = None; false }
        }
    }
}

fn notify(app: &AppHandle, phase: &'static str, message: impl Into<String>) {
    emit_task(app, TaskEvent {
        kind: "install", phase, message: message.into(), percent: None, bytes_per_second: None,
        task_id: None, bytes_downloaded: None, total_bytes: None,
    });
}

fn command_output(program: &str, args: &[String]) -> Result<String, String> {
    let output = Command::new(program).args(args).output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("Команда {program} завершилась с ошибкой."));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn python_310() -> Result<(String, Vec<String>), String> {
    let candidates: Vec<(&str, Vec<&str>)> = if cfg!(windows) {
        vec![("py", vec!["-3.10"]), ("python", vec![]), ("python3.10", vec![])]
    } else {
        vec![("python3.10", vec![]), ("python3", vec![]), ("python", vec![])]
    };
    for (program, args) in candidates {
        let output = Command::new(program)
            .args(&args)
            .args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
            .output();
        if let Ok(output) = output {
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "3.10" {
                return Ok((program.to_owned(), args.into_iter().map(str::to_owned).collect()));
            }
        }
    }
    Err("Не найден Python 3.10. Установите его и перезапустите Flow.".to_owned())
}

fn pipe_logs(app: AppHandle, phase: &'static str, mut reader: impl Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 { break; }
            notify(&app, phase, String::from_utf8_lossy(&buffer[..count]).into_owned());
        }
    })
}

fn run_logged(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path, phase: &'static str) -> Result<(), String> {
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let mut child = Command::new(program)
        .args(args).current_dir(cwd)
        .env("PYTHONUNBUFFERED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| format!("Не удалось запустить {program}: {error}"))?;
    if let Ok(mut pid) = control.pid.lock() { *pid = Some(child.id()); }
    let stdout = child.stdout.take().map(|pipe| pipe_logs(app.clone(), phase, pipe));
    let stderr = child.stderr.take().map(|pipe| pipe_logs(app.clone(), phase, pipe));
    let status = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            let _ = child.kill();
            break child.wait().map_err(|error| error.to_string())?;
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? { break status; }
        thread::sleep(Duration::from_millis(100));
    };
    if let Ok(mut pid) = control.pid.lock() { *pid = None; }
    if let Some(handle) = stdout { let _ = handle.join(); }
    if let Some(handle) = stderr { let _ = handle.join(); }
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    if !status.success() { return Err(format!("Команда завершилась с кодом {:?}.", status.code())); }
    Ok(())
}

fn install(app: &AppHandle, control: &InstallControl, parent: &Path) -> Result<(), String> {
    notify(app, "checking", "Проверка Git и Python 3.10…");
    command_output("git", &["--version".to_owned()]).map_err(|_| "Git не найден. Установите Git и перезапустите Flow.".to_owned())?;
    let (python, prefix) = python_310()?;
    let directory = parent.join("sd-webui");
    notify(app, "cloning", format!("Папка пакета: {}", directory.display()));
    if directory.exists() {
        if !directory.join(".git").is_dir() { return Err("Папка sd-webui занята другим содержимым.".to_owned()); }
        let remote = command_output("git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "remote".to_owned(), "get-url".to_owned(), "origin".to_owned()])?;
        if remote.trim_end_matches('/').trim_end_matches(".git").to_lowercase() != WEBUI_REPO.trim_end_matches(".git").to_lowercase() {
            return Err("В выбранной папке находится другой Git-репозиторий.".to_owned());
        }
        notify(app, "cloning", "Найден существующий репозиторий WebUI; продолжаю установку.");
    } else {
        run_logged(app, control, "git", &["clone".to_owned(), "--progress".to_owned(), WEBUI_REPO.to_owned(), directory.to_string_lossy().into_owned()], parent, "cloning")?;
    }
    let venv = directory.join("venv");
    let venv_python = if cfg!(windows) { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") };
    if !venv_python.is_file() {
        notify(app, "venv", "Создание виртуального окружения…");
        let mut args = prefix;
        args.extend(["-m".to_owned(), "venv".to_owned(), venv.to_string_lossy().into_owned()]);
        run_logged(app, control, &python, &args, &directory, "venv")?;
    }
    if !directory.join("launch.py").is_file() { return Err("Файл launch.py не найден.".to_owned()); }
    notify(app, "dependencies", "Установка зависимостей WebUI…");
    run_logged(app, control, &venv_python.to_string_lossy(), &["-u".to_owned(), "launch.py".to_owned(), "--exit".to_owned(), "--skip-torch-cuda-test".to_owned()], &directory, "dependencies")?;
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let version = command_output("git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "describe".to_owned(), "--tags".to_owned(), "--always".to_owned()]).unwrap_or_else(|_| "Установлен".to_owned());
    #[cfg(windows)] {
        let settings = directory.join("webui.settings.bat");
        if !settings.exists() {
            fs::write(&settings, format!("@echo off\r\nset PYTHON=\"{}\"\r\nset STABLE_DIFFUSION_REPO={}\r\n", venv_python.display(), SD_REPO)).map_err(|error| error.to_string())?;
        }
    }
    fs::write(directory.join(".flow-installed.json"), json!({ "name": "AUTOMATIC1111 WebUI", "version": version }).to_string()).map_err(|error| error.to_string())?;
    notify(app, "done", "AUTOMATIC1111 WebUI установлен.");
    Ok(())
}

#[tauri::command]
pub(crate) async fn install_web_ui(app: AppHandle, state: State<'_, Arc<InstallControl>>, installation_dir: String) -> Result<(), String> {
    let selected = crate::paths::subprocess_path(&PathBuf::from(installation_dir.trim()));
    if !selected.is_absolute() || !selected.is_dir() { return Err("Выберите существующую папку установки.".to_owned()); }
    // Reject stale UI state: the path passed to Rust must be the saved selection.
    let selected = crate::paths::subprocess_path(&selected.canonicalize().map_err(|error| error.to_string())?);
    let saved = crate::paths::subprocess_path(&crate::installation_dir(&app)?.canonicalize().map_err(|error| error.to_string())?);
    if selected != saved {
        return Err("Папка установки изменилась. Повторите выбор.".to_owned());
    }
    if state.running.swap(true, Ordering::SeqCst) { return Err("Установка уже выполняется.".to_owned()); }
    state.cancelled.store(false, Ordering::SeqCst);
    let control = Arc::clone(&*state);
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = install(&app, &control, &selected);
        if let Err(error) = outcome {
            let phase = if control.cancelled.load(Ordering::SeqCst) { "cancelled" } else { "error" };
            notify(&app, phase, error);
        }
        control.running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_install(state: State<'_, Arc<InstallControl>>) {
    state.cancelled.store(true, Ordering::SeqCst);
    if let Ok(slot) = state.pid.lock() {
        if let Some(pid) = *slot {
            #[cfg(windows)] { let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output(); }
            #[cfg(not(windows))] { let _ = pid; }
        }
    }
}

#[tauri::command]
pub(crate) fn launch_web_ui(app: AppHandle, webui: State<'_, WebuiControl>) -> Result<(), String> {
    let directory = installation_dir(&app)?.join("sd-webui");
    if !directory.join(".flow-installed.json").is_file() { return Err("Сначала установите пакет.".to_owned()); }
    if webui.is_running() { return Ok(()); }
    let mut command = if cfg!(windows) {
        let script = directory.join("webui-user.bat");
        if !script.is_file() { return Err("Файл запуска WebUI не найден.".to_owned()); }
        let mut command = Command::new("cmd.exe");
        command.arg("/c").arg(script);
        command
    } else {
        let script = directory.join("webui.sh");
        if !script.is_file() { return Err("Файл запуска WebUI не найден.".to_owned()); }
        let mut command = Command::new("bash");
        command.arg(script);
        command
    };
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
    }
    command.current_dir(&directory).env("STABLE_DIFFUSION_REPO", SD_REPO);
    let child = command.spawn().map_err(|error| error.to_string())?;
    *webui.0.lock().map_err(|_| "Ошибка доступа к процессу WebUI.".to_owned())? = Some(child);
    Ok(())
}

#[tauri::command]
pub(crate) fn open_package_settings(app: AppHandle) -> Result<(), String> {
    let directory = installation_dir(&app)?.join("sd-webui");
    if !directory.join(".flow-installed.json").is_file() { return Err("Сначала установите пакет.".to_owned()); }
    #[cfg(windows)] let mut command = { let mut cmd = Command::new("notepad.exe"); cmd.arg(directory.join("webui.settings.bat")); cmd };
    #[cfg(target_os = "macos")] let mut command = { let mut cmd = Command::new("open"); cmd.arg(&directory); cmd };
    #[cfg(all(not(windows), not(target_os = "macos")))] let mut command = { let mut cmd = Command::new("xdg-open"); cmd.arg(&directory); cmd };
    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}
