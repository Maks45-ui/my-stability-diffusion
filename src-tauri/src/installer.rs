use crate::{emit_task, installation_dir, TaskEvent};
use serde_json::json;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
pub(crate) struct WebuiControl {
    running: Arc<AtomicBool>,
}

impl WebuiControl {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

fn notify_kind(app: &AppHandle, kind: &'static str, phase: &'static str, message: impl Into<String>) {
    emit_task(app, TaskEvent {
        kind, phase, message: message.into(), percent: None, bytes_per_second: None,
        task_id: None, bytes_downloaded: None, total_bytes: None,
    });
}

fn notify(app: &AppHandle, phase: &'static str, message: impl Into<String>) {
    notify_kind(app, "install", phase, message);
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))] let _ = command;
}

fn emit_output(app: &AppHandle, output: &[u8]) {
    for line in String::from_utf8_lossy(output).split(|character| character == '\r' || character == '\n') {
        if !line.trim().is_empty() { notify(app, "log", line.to_owned()); }
    }
}

fn command_output(app: &AppHandle, program: &str, args: &[String]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    hide_console(&mut command);
    let output = command.output().map_err(|error| error.to_string())?;
    emit_output(app, &output.stdout);
    emit_output(app, &output.stderr);
    if !output.status.success() {
        return Err(format!("Команда {program} завершилась с кодом {:?}.", output.status.code()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn python_310(app: &AppHandle) -> Result<(String, Vec<String>), String> {
    let candidates: Vec<(&str, Vec<&str>)> = if cfg!(windows) {
        vec![("py", vec!["-3.10"]), ("python", vec![]), ("python3.10", vec![])]
    } else {
        vec![("python3.10", vec![]), ("python3", vec![]), ("python", vec![])]
    };
    for (program, args) in candidates {
        let mut command = Command::new(program);
        command.args(&args)
            .args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
            .stdin(Stdio::null());
        hide_console(&mut command);
        let output = command.output();
        if let Ok(output) = output {
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "3.10" {
                emit_output(app, &output.stdout);
                notify(app, "checking", format!("Python 3.10 найден: {program}"));
                return Ok((program.to_owned(), args.into_iter().map(str::to_owned).collect()));
            }
            emit_output(app, &output.stderr);
        }
    }
    Err("Не найден Python 3.10. Установите его и перезапустите Flow.".to_owned())
}

fn pipe_logs(app: AppHandle, kind: &'static str, phase: &'static str, mut reader: impl Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        let mut line = Vec::new();
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 { break; }
            for &byte in &buffer[..count] {
                if byte == b'\n' || byte == b'\r' {
                    if !line.is_empty() {
                        notify_kind(&app, kind, phase, String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                } else {
                    line.push(byte);
                    if line.len() >= 16_384 {
                        notify_kind(&app, kind, phase, String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                }
            }
        }
        if !line.is_empty() { notify_kind(&app, kind, phase, String::from_utf8_lossy(&line).into_owned()); }
    })
}

fn run_logged(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path) -> Result<(), String> {
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd)
        .env("PYTHONUNBUFFERED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|error| format!("Не удалось запустить {program}: {error}"))?;
    if let Ok(mut pid) = control.pid.lock() { *pid = Some(child.id()); }
    let stdout = child.stdout.take().map(|pipe| pipe_logs(app.clone(), "install", "log", pipe));
    let stderr = child.stderr.take().map(|pipe| pipe_logs(app.clone(), "install", "log", pipe));
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
    command_output(app, "git", &["--version".to_owned()]).map_err(|_| "Git не найден. Установите Git и перезапустите Flow.".to_owned())?;
    let (python, prefix) = python_310(app)?;
    let directory = parent.join("sd-webui");
    notify(app, "cloning", format!("Папка пакета: {}", directory.display()));
    if directory.exists() {
        if !directory.join(".git").is_dir() { return Err("Папка sd-webui занята другим содержимым.".to_owned()); }
        let remote = command_output(app, "git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "remote".to_owned(), "get-url".to_owned(), "origin".to_owned()])?;
        if remote.trim_end_matches('/').trim_end_matches(".git").to_lowercase() != WEBUI_REPO.trim_end_matches(".git").to_lowercase() {
            return Err("В выбранной папке находится другой Git-репозиторий.".to_owned());
        }
        notify(app, "cloning", "Найден существующий репозиторий WebUI; продолжаю установку.");
    } else {
        run_logged(app, control, "git", &["clone".to_owned(), "--progress".to_owned(), WEBUI_REPO.to_owned(), directory.to_string_lossy().into_owned()], parent)?;
    }
    let venv = directory.join("venv");
    let venv_python = if cfg!(windows) { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") };
    if !venv_python.is_file() {
        notify(app, "venv", "Создание виртуального окружения…");
        let mut args = prefix;
        args.extend(["-m".to_owned(), "venv".to_owned(), venv.to_string_lossy().into_owned()]);
        run_logged(app, control, &python, &args, &directory)?;
    }
    if !directory.join("launch.py").is_file() { return Err("Файл launch.py не найден.".to_owned()); }
    notify(app, "dependencies", "Установка зависимостей WebUI…");
    run_logged(app, control, &venv_python.to_string_lossy(), &["-u".to_owned(), "launch.py".to_owned(), "--exit".to_owned(), "--skip-torch-cuda-test".to_owned()], &directory)?;
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let version = command_output(app, "git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "describe".to_owned(), "--tags".to_owned(), "--always".to_owned()]).unwrap_or_else(|_| "Установлен".to_owned());
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
pub(crate) fn cancel_install(app: AppHandle, state: State<'_, Arc<InstallControl>>) {
    state.cancelled.store(true, Ordering::SeqCst);
    if let Ok(slot) = state.pid.lock() {
        if let Some(pid) = *slot {
            #[cfg(windows)] {
                let mut command = Command::new("taskkill");
                command.args(["/PID", &pid.to_string(), "/T", "/F"]);
                hide_console(&mut command);
                if let Ok(output) = command.output() {
                    emit_output(&app, &output.stdout);
                    emit_output(&app, &output.stderr);
                }
            }
            #[cfg(not(windows))] { let _ = pid; }
        }
    }
}

#[tauri::command]
pub(crate) fn launch_web_ui(app: AppHandle, webui: State<'_, WebuiControl>) -> Result<(), String> {
    let directory = installation_dir(&app)?.join("sd-webui");
    if !directory.join(".flow-installed.json").is_file() { return Err("Сначала установите пакет.".to_owned()); }
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
    command.current_dir(&directory)
        .env("PYTHONUNBUFFERED", "1")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut command);
    if webui.running.swap(true, Ordering::SeqCst) { return Ok(()); }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            webui.running.store(false, Ordering::SeqCst);
            return Err(format!("Не удалось запустить WebUI: {error}"));
        }
    };
    notify_kind(&app, "launch", "running", "WebUI запускается…");
    let running = Arc::clone(&webui.running);
    thread::spawn(move || {
        let stdout = child.stdout.take().map(|pipe| pipe_logs(app.clone(), "launch", "log", pipe));
        let stderr = child.stderr.take().map(|pipe| pipe_logs(app.clone(), "launch", "log", pipe));
        let result = child.wait();
        if let Some(handle) = stdout { let _ = handle.join(); }
        if let Some(handle) = stderr { let _ = handle.join(); }
        running.store(false, Ordering::SeqCst);
        match result {
            Ok(status) if status.success() => notify_kind(&app, "launch", "done", "WebUI остановлен."),
            Ok(status) => notify_kind(&app, "launch", "error", format!("WebUI завершился с кодом {:?}.", status.code())),
            Err(error) => notify_kind(&app, "launch", "error", format!("Ошибка процесса WebUI: {error}")),
        }
    });
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
