use crate::{emit_task, installation_dir, TaskEvent};
use serde_json::json;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    collections::HashSet,
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for WindowsJob {}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); }
    }
}

#[cfg(windows)]
impl WindowsJob {
    fn terminate(&self) -> Result<(), String> {
        let result = unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
        if result == 0 { Err(format!("Не удалось завершить группу WebUI: {}", std::io::Error::last_os_error())) }
        else { Ok(()) }
    }
}

#[cfg(windows)]
fn webui_job(child: &std::process::Child) -> Result<WindowsJob, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
    // The job groups WebUI's descendants for an explicit Stop. Closing Flow must not kill Python.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() { return Err(format!("Не удалось создать группу WebUI: {}", std::io::Error::last_os_error())); }
    let job = WindowsJob(handle);
    let assigned = unsafe { AssignProcessToJobObject(handle, child.as_raw_handle() as _) };
    if assigned == 0 { return Err(format!("Не удалось добавить WebUI в группу процессов: {}", std::io::Error::last_os_error())); }
    Ok(job)
}

const WEBUI_REPO: &str = "https://github.com/AUTOMATIC1111/stable-diffusion-webui.git";
const SD_REPO: &str = "https://github.com/w-e-w/stablediffusion.git";
const CLIP_REPO: &str = "https://github.com/openai/CLIP.git";
const CLIP_REVISION: &str = "d50d76daa670286dd6cacf3bcd80b5e4823fc8e1";
const OPENCLIP_REPO: &str = "https://github.com/mlfoundations/open_clip.git";
const OPENCLIP_REVISION: &str = "bb6e834e9c70d9c27d0dc3ecedeebeaeb1ffad6b";

#[derive(Default)]
pub(crate) struct InstallControl {
    running: AtomicBool,
    cancelled: AtomicBool,
    pid: Mutex<Option<u32>>,
}

impl InstallControl {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[derive(Default, Clone)]
pub(crate) struct WebuiControl {
    running: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    pid: Arc<Mutex<Option<u32>>>,
    ready_url: Arc<Mutex<Option<String>>>,
    ready_pending: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    #[cfg(windows)]
    job: Arc<Mutex<Option<WindowsJob>>>,
}

impl WebuiControl {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub(crate) fn was_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub(crate) fn ready_url(&self) -> Option<String> {
        if self.exited.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) { return None; }
        self.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()).clone()
    }
}

const WEBUI_OPEN_DELAY: Duration = Duration::from_millis(1500);

fn notify_kind(app: &AppHandle, kind: &'static str, phase: &'static str, message: impl Into<String>) {
    emit_task(app, TaskEvent {
        kind, phase, message: message.into(), percent: None, bytes_per_second: None,
        task_id: None, bytes_downloaded: None, total_bytes: None, url: None,
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

fn use_local_clip_sources(command: &mut Command, directory: &Path) {
    for (variable, folder) in [("CLIP_PACKAGE", "clip"), ("OPENCLIP_PACKAGE", "open_clip")] {
        let source = directory.join("repositories").join(folder);
        if source.join(".git").is_dir() {
            // launch_utils builds a shell command, so the path must stay quoted.
            command.env(variable, format!("\"{}\"", source.display()));
        }
    }
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

fn python_module_installed(python: &Path, module: &str) -> Result<bool, String> {
    let mut command = Command::new(python);
    command.args(["-c", "import importlib.util, sys; print(int(importlib.util.find_spec(sys.argv[1]) is not None))", module])
        .stdin(Stdio::null());
    hide_console(&mut command);
    let output = command.output().map_err(|error| format!("Не удалось проверить модуль {module}: {error}"))?;
    if !output.status.success() {
        return Err(format!("Не удалось проверить модуль {module}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn parse_webui_ready_url(line: &str) -> Option<String> {
    let marker = "Running on local URL:";
    let (_, suffix) = line.split_once(marker)?;
    let (_, port_text) = suffix.split_once("http://127.0.0.1:")?;
    let digits: String = port_text.chars().take_while(char::is_ascii_digit).collect();
    let port = digits.parse::<u16>().ok().filter(|port| *port != 0)?;
    Some(format!("http://127.0.0.1:{port}"))
}

fn emit_pipe_line(app: &AppHandle, kind: &'static str, phase: &'static str, line: &[u8], locked_file: Option<&AtomicBool>, webui: Option<&WebuiControl>) {
    let message = String::from_utf8_lossy(line).into_owned();
    if let Some(locked_file) = locked_file {
        let lower = message.to_ascii_lowercase();
        if lower.contains(".deleteme") || lower.contains("winerror 32") {
            locked_file.store(true, Ordering::SeqCst);
        }
    }
    notify_kind(app, kind, phase, message);
    if let Some(webui) = webui {
        if let Some(url) = parse_webui_ready_url(&String::from_utf8_lossy(line)) {
            if !webui.is_running() || webui.is_stopping() || webui.exited.load(Ordering::SeqCst)
                || webui.ready_pending.swap(true, Ordering::SeqCst) { return; }
            notify_kind(app, "launch", "stabilizing", "Локальный адрес получен; проверка стабильности запуска 1,5 с…");
            let app = app.clone();
            let control = webui.clone();
            let generation = control.generation.load(Ordering::SeqCst);
            thread::spawn(move || {
                thread::sleep(WEBUI_OPEN_DELAY);
                if !control.is_running() || control.is_stopping() || control.exited.load(Ordering::SeqCst)
                    || control.generation.load(Ordering::SeqCst) != generation { return; }
                let mut ready = control.ready_url.lock().unwrap_or_else(|poison| poison.into_inner());
                if control.exited.load(Ordering::SeqCst) || control.generation.load(Ordering::SeqCst) != generation { return; }
                *ready = Some(url.clone());
                drop(ready);
                if control.exited.load(Ordering::SeqCst) { return; }
                emit_task(&app, TaskEvent {
                    kind: "launch", phase: "ready", message: format!("WebUI доступен: {url}"), url: Some(url),
                    percent: None, bytes_per_second: None, task_id: None,
                    bytes_downloaded: None, total_bytes: None,
                });
            });
        }
    }
}

fn pipe_logs(app: AppHandle, kind: &'static str, phase: &'static str, mut reader: impl Read + Send + 'static, locked_file: Option<Arc<AtomicBool>>, webui: Option<WebuiControl>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        let mut line = Vec::new();
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 { break; }
            for &byte in &buffer[..count] {
                if byte == b'\n' || byte == b'\r' {
                    if !line.is_empty() {
                        emit_pipe_line(&app, kind, phase, &line, locked_file.as_deref(), webui.as_ref());
                        line.clear();
                    }
                } else {
                    line.push(byte);
                    if line.len() >= 16_384 {
                        emit_pipe_line(&app, kind, phase, &line, locked_file.as_deref(), webui.as_ref());
                        line.clear();
                    }
                }
            }
        }
        if !line.is_empty() { emit_pipe_line(&app, kind, phase, &line, locked_file.as_deref(), webui.as_ref()); }
    })
}

fn tail_webui_log(app: AppHandle, mut reader: fs::File, control: WebuiControl) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        let mut line = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) if control.exited.load(Ordering::SeqCst) => break,
                Ok(0) => thread::sleep(Duration::from_millis(50)),
                Ok(count) => {
                    for &byte in &buffer[..count] {
                        if byte == b'\n' || byte == b'\r' {
                            if !line.is_empty() {
                                emit_pipe_line(&app, "launch", "log", &line, None, Some(&control));
                                line.clear();
                            }
                        } else {
                            line.push(byte);
                            if line.len() >= 16_384 {
                                emit_pipe_line(&app, "launch", "log", &line, None, Some(&control));
                                line.clear();
                            }
                        }
                    }
                }
                Err(error) => {
                    notify_kind(&app, "launch", "log", format!("Ошибка чтения журнала WebUI: {error}"));
                    break;
                }
            }
        }
        if !line.is_empty() { emit_pipe_line(&app, "launch", "log", &line, None, Some(&control)); }
    })
}

fn run_logged(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path, locked_file: Option<Arc<AtomicBool>>) -> Result<(), String> {
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd)
        .env("PYTHONUNBUFFERED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    use_local_clip_sources(&mut command, cwd);
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|error| format!("Не удалось запустить {program}: {error}"))?;
    if let Ok(mut pid) = control.pid.lock() { *pid = Some(child.id()); }
    let stdout = child.stdout.take().map(|pipe| pipe_logs(app.clone(), "install", "log", pipe, locked_file.clone(), None));
    let stderr = child.stderr.take().map(|pipe| pipe_logs(app.clone(), "install", "log", pipe, locked_file, None));
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

fn run_logged_retry_locked(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path) -> Result<(), String> {
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        let locked_file = Arc::new(AtomicBool::new(false));
        match run_logged(app, control, program, args, cwd, Some(Arc::clone(&locked_file))) {
            Ok(()) => return Ok(()),
            Err(_) if cfg!(windows) && locked_file.load(Ordering::SeqCst) && attempt < MAX_ATTEMPTS && !control.cancelled.load(Ordering::SeqCst) => {
                notify(app, "dependencies", format!("Windows заблокировал временный файл pip; повтор команды ({}/{MAX_ATTEMPTS})…", attempt + 1));
                thread::sleep(Duration::from_secs(2));
            }
            Err(error) if locked_file.load(Ordering::SeqCst) => {
                return Err(format!("{error} Временный файл .deleteme остался заблокирован после {attempt} попыток."));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("цикл всегда возвращает результат последней попытки")
}

pub(crate) fn parse_webui_arguments(entries: &[String]) -> Result<Vec<String>, String> {
    if entries.len() > 32 { return Err("Можно сохранить не более 32 аргументов WebUI.".to_owned()); }
    let mut parsed = Vec::with_capacity(entries.len() * 2);
    let mut names = HashSet::new();
    for raw in entries {
        let entry = raw.trim();
        if entry.is_empty() || entry.len() > 256 || entry.chars().any(char::is_control) {
            return Err("Аргумент должен содержать от 1 до 256 символов без переноса строки.".to_owned());
        }
        let flag_end = entry.char_indices().find(|(_, character)| character.is_whitespace()).map_or(entry.len(), |(index, _)| index);
        let flag = &entry[..flag_end];
        let name = flag.split_once('=').map_or(flag, |(name, _)| name);
        if !name.starts_with("--") || !name[2..].bytes().next().is_some_and(|byte| byte.is_ascii_alphabetic())
            || !name[2..].bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
            return Err(format!("Некорректный аргумент: {entry}. Используйте формат --имя или --имя значение."));
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Аргумент {name} уже добавлен."));
        }
        let value = entry[flag_end..].trim();
        if let Some((_, inline_value)) = flag.split_once('=') {
            if inline_value.is_empty() || !value.is_empty() {
                return Err(format!("Укажите одно значение для {name}."));
            }
        }
        parsed.push(flag.to_owned());
        if !value.is_empty() {
            let unquoted = value.strip_prefix('"').and_then(|part| part.strip_suffix('"')).unwrap_or(value);
            if unquoted.is_empty() || unquoted.starts_with("--") || unquoted.contains('"') {
                return Err(format!("Некорректное значение для {name}. Добавляйте флаги по одному."));
            }
            parsed.push(unquoted.to_owned());
        }
    }
    Ok(parsed)
}

fn ensure_local_clip_source(app: &AppHandle, control: &InstallControl, directory: &Path, folder: &str, url: &str, revision: &str) -> Result<PathBuf, String> {
    let repositories = directory.join("repositories");
    fs::create_dir_all(&repositories).map_err(|error| error.to_string())?;
    let source = repositories.join(folder);
    let source_arg = source.to_string_lossy().into_owned();
    if !source.exists() {
        notify(app, "dependencies", format!("Загрузка исходников {folder}…"));
        run_logged(app, control, "git", &["clone".to_owned(), "--progress".to_owned(), url.to_owned(), source_arg.clone()], directory, None)?;
    }
    if !source.join(".git").is_dir() {
        return Err(format!("Папка {} занята другим содержимым.", source.display()));
    }
    let origin = command_output(app, "git", &["-C".to_owned(), source_arg.clone(), "remote".to_owned(), "get-url".to_owned(), "origin".to_owned()])?;
    if origin.trim_end_matches('/').trim_end_matches(".git").to_lowercase() != url.trim_end_matches(".git").to_lowercase() {
        return Err(format!("В папке {} находится другой Git-репозиторий.", source.display()));
    }
    let current = command_output(app, "git", &["-C".to_owned(), source_arg.clone(), "rev-parse".to_owned(), "HEAD".to_owned()])?;
    if current != revision {
        let changes = command_output(app, "git", &["-C".to_owned(), source_arg.clone(), "status".to_owned(), "--porcelain".to_owned()])?;
        if !changes.is_empty() { return Err(format!("В папке {} есть несохранённые изменения; обновление остановлено.", source.display())); }
        let revision_exists = command_output(app, "git", &["-C".to_owned(), source_arg.clone(), "cat-file".to_owned(), "-e".to_owned(), format!("{revision}^{{commit}}")]).is_ok();
        if !revision_exists {
            run_logged(app, control, "git", &["-C".to_owned(), source_arg.clone(), "fetch".to_owned(), "--no-tags".to_owned(), "origin".to_owned(), revision.to_owned()], directory, None)?;
        }
        run_logged(app, control, "git", &["-C".to_owned(), source_arg, "checkout".to_owned(), "--detach".to_owned(), revision.to_owned()], directory, None)?;
    }
    Ok(source)
}

fn install_local_clip(app: &AppHandle, control: &InstallControl, python: &Path, directory: &Path, module: &str, folder: &str, url: &str, revision: &str) -> Result<(), String> {
    if python_module_installed(python, module)? {
        notify(app, "dependencies", format!("{module} уже установлен; пропускаю."));
        return Ok(());
    }
    let source = ensure_local_clip_source(app, control, directory, folder, url, revision)?;
    notify(app, "dependencies", format!("Установка {module} из локального репозитория…"));
    run_logged_retry_locked(app, control, &python.to_string_lossy(), &[
        "-m".to_owned(), "pip".to_owned(), "install".to_owned(),
        "--no-build-isolation".to_owned(), "--no-deps".to_owned(),
        "-e".to_owned(), source.to_string_lossy().into_owned(),
    ], directory)?;
    if !python_module_installed(python, module)? {
        return Err(format!("Модуль {module} не появился в виртуальном окружении после установки."));
    }
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
        run_logged(app, control, "git", &["clone".to_owned(), "--progress".to_owned(), WEBUI_REPO.to_owned(), directory.to_string_lossy().into_owned()], parent, None)?;
    }
    let venv = directory.join("venv");
    let venv_python = if cfg!(windows) { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") };
    if !venv_python.is_file() {
        notify(app, "venv", "Создание виртуального окружения…");
        let mut args = prefix;
        args.extend(["-m".to_owned(), "venv".to_owned(), venv.to_string_lossy().into_owned()]);
        run_logged(app, control, &python, &args, &directory, None)?;
    }
    if !directory.join("launch.py").is_file() { return Err("Файл launch.py не найден.".to_owned()); }
    notify(app, "dependencies", "Обновление pip в виртуальном окружении…");
    run_logged_retry_locked(app, control, &venv_python.to_string_lossy(), &["-m".to_owned(), "pip".to_owned(), "install".to_owned(), "--upgrade".to_owned(), "pip".to_owned()], &directory)?;
    let clip_missing = !python_module_installed(&venv_python, "clip")?;
    let openclip_missing = !python_module_installed(&venv_python, "open_clip")?;
    if clip_missing || openclip_missing {
        notify(app, "dependencies", "Подготовка сборщика Python для CLIP…");
        run_logged_retry_locked(app, control, &venv_python.to_string_lossy(), &["-m".to_owned(), "pip".to_owned(), "install".to_owned(), "wheel".to_owned(), "setuptools==69.5.1".to_owned()], &directory)?;
    }
    if clip_missing { install_local_clip(app, control, &venv_python, &directory, "clip", "clip", CLIP_REPO, CLIP_REVISION)?; }
    if openclip_missing { install_local_clip(app, control, &venv_python, &directory, "open_clip", "open_clip", OPENCLIP_REPO, OPENCLIP_REVISION)?; }
    notify(app, "dependencies", "Установка зависимостей WebUI…");
    // launch.py installs requirements_versions.txt with WebUI's own torch configuration.
    run_logged_retry_locked(app, control, &venv_python.to_string_lossy(), &["-u".to_owned(), "launch.py".to_owned(), "--exit".to_owned(), "--skip-torch-cuda-test".to_owned()], &directory)?;
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    if !python_module_installed(&venv_python, "clip")? || !python_module_installed(&venv_python, "open_clip")? {
        return Err("После установки WebUI модули CLIP или open_clip недоступны в виртуальном окружении.".to_owned());
    }
    let version = command_output(app, "git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "describe".to_owned(), "--tags".to_owned(), "--always".to_owned()]).unwrap_or_else(|_| "Установлен".to_owned());
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
    if !directory.join("launch.py").is_file() { return Err("Файл запуска WebUI не найден.".to_owned()); }
    let python = if cfg!(windows) { directory.join("venv/Scripts/python.exe") } else { directory.join("venv/bin/python") };
    if !python.is_file() { return Err("Виртуальное окружение WebUI не найдено.".to_owned()); }
    let arguments = parse_webui_arguments(&crate::webui_arguments(&app)?)?;
    // webui-user.bat clears COMMANDLINE_ARGS and does not forward its own arguments.
    // Passing args directly to launch.py also avoids shell quoting and a visible CMD window.
    let mut command = Command::new(python);
    command.arg("-u").arg("launch.py").args(arguments);
    use_local_clip_sources(&mut command, &directory);
    command.current_dir(&directory)
        .env("PYTHONUNBUFFERED", "1")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        // The saved flags are passed as argv; inherited COMMANDLINE_ARGS must not repeat them.
        .env_remove("COMMANDLINE_ARGS")
        .stdin(Stdio::null());
    hide_console(&mut command);
    if webui.running.swap(true, Ordering::SeqCst) { return Err("WebUI уже запущен или запускается.".to_owned()); }
    webui.stopping.store(false, Ordering::SeqCst);
    webui.stopped.store(false, Ordering::SeqCst);
    webui.exited.store(false, Ordering::SeqCst);
    webui.ready_pending.store(false, Ordering::SeqCst);
    webui.generation.fetch_add(1, Ordering::SeqCst);
    *webui.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    // Python writes to a real file. A WebView2/Flow crash cannot close its stdout pipe.
    let log_setup = (|| -> Result<(fs::File, fs::File, PathBuf), String> {
        let log_dir = crate::data_dir(&app)?.join("logs");
        fs::create_dir_all(&log_dir).map_err(|error| error.to_string())?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let log_path = log_dir.join(format!("webui-{}-{stamp}.log", std::process::id()));
        let writer = fs::OpenOptions::new().append(true).create_new(true).open(&log_path)
            .map_err(|error| format!("Не удалось создать журнал WebUI: {error}"))?;
        let reader = fs::File::open(&log_path).map_err(|error| format!("Не удалось открыть журнал WebUI: {error}"))?;
        Ok((writer, reader, log_path))
    })();
    let (writer, reader, log_path) = match log_setup {
        Ok(files) => files,
        Err(error) => { webui.running.store(false, Ordering::SeqCst); return Err(error); }
    };
    let stdout = match writer.try_clone() {
        Ok(file) => file,
        Err(error) => { webui.running.store(false, Ordering::SeqCst); return Err(format!("Не удалось открыть вывод WebUI: {error}")); }
    };
    command.stdout(Stdio::from(stdout)).stderr(Stdio::from(writer));
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            webui.running.store(false, Ordering::SeqCst);
            return Err(format!("Не удалось запустить WebUI: {error}"));
        }
    };
    #[cfg(windows)] {
        let job = match webui_job(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                webui.running.store(false, Ordering::SeqCst);
                return Err(error);
            }
        };
        *webui.job.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(job);
    }
    *webui.pid.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(child.id());
    notify_kind(&app, "launch", "starting", "WebUI запускается…");
    notify_kind(&app, "launch", "log", format!("Журнал WebUI: {}", log_path.display()));
    let control = (*webui).clone();
    thread::spawn(move || {
        let logger = tail_webui_log(app.clone(), reader, control.clone());
        let result = child.wait();
        control.exited.store(true, Ordering::SeqCst);
        let was_stopping = control.stopping.swap(false, Ordering::SeqCst);
        // Terminate remaining descendants before the log tailer drains the final output.
        #[cfg(windows)] {
            if let Some(job) = control.job.lock().unwrap_or_else(|poison| poison.into_inner()).take() {
                let _ = job.terminate();
            }
        }
        let _ = logger.join();
        let was_ready = control.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()).is_some();
        *control.pid.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        *control.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        control.stopped.store(true, Ordering::SeqCst);
        control.running.store(false, Ordering::SeqCst);
        match result {
            Ok(_) if was_stopping => notify_kind(&app, "launch", "done", "WebUI остановлен пользователем."),
            Ok(status) if status.success() && was_ready => notify_kind(&app, "launch", "done", "WebUI остановлен."),
            Ok(status) if status.success() => notify_kind(&app, "launch", "error", "WebUI завершился до появления локального адреса."),
            Ok(status) if status.code() == Some(-1073741819) => notify_kind(&app, "launch", "error", "WebUI аварийно завершился: 0xC0000005 (нарушение доступа). Проверьте последние строки лога WebUI и журнал событий Windows."),
            Ok(status) => notify_kind(&app, "launch", "error", format!("WebUI завершился с кодом {:?}.", status.code())),
            Err(error) => notify_kind(&app, "launch", "error", format!("Ошибка процесса WebUI: {error}")),
        }
    });
    Ok(())
}

#[tauri::command]
pub(crate) async fn stop_web_ui(app: AppHandle, webui: State<'_, WebuiControl>) -> Result<(), String> {
    if !webui.is_running() || webui.exited.load(Ordering::SeqCst) { return Err("WebUI уже остановлен.".to_owned()); }
    let pid = (*webui.pid.lock().unwrap_or_else(|poison| poison.into_inner()))
        .ok_or("Процесс WebUI ещё запускается. Повторите остановку через секунду.")?;
    if webui.stopping.swap(true, Ordering::SeqCst) { return Ok(()); }
    if webui.exited.load(Ordering::SeqCst) {
        webui.stopping.store(false, Ordering::SeqCst);
        return Err("WebUI уже остановлен.".to_owned());
    }
    notify_kind(&app, "launch", "stopping", "Остановка WebUI…");
    let control = (*webui).clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("taskkill");
            command.args(["/PID", &pid.to_string(), "/T", "/F"]);
            hide_console(&mut command);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("kill");
            command.args(["-TERM", &pid.to_string()]);
            command
        };
        let output = command.output();
        #[cfg(windows)]
        let managed = control.job.lock().unwrap_or_else(|poison| poison.into_inner()).take()
            .is_some_and(|job| job.terminate().is_ok());
        #[cfg(not(windows))]
        let managed = false;
        let result = match output {
            Ok(output) if output.status.success() || managed || control.exited.load(Ordering::SeqCst) || !control.is_running() => Ok(()),
            Ok(output) => Err(format!("Не удалось остановить WebUI: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(_) if managed || control.exited.load(Ordering::SeqCst) || !control.is_running() => Ok(()),
            Err(error) => Err(format!("Не удалось остановить WebUI: {error}")),
        };
        if result.is_err() { control.stopping.store(false, Ordering::SeqCst); }
        result
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn open_web_ui(app: AppHandle, webui: State<'_, WebuiControl>) -> Result<(), String> {
    if !webui.is_running() || webui.is_stopping() || webui.exited.load(Ordering::SeqCst) { return Err("WebUI сейчас недоступен.".to_owned()); }
    let url = webui.ready_url().ok_or("WebUI ещё запускается.")?;
    app.opener().open_url(url, None::<&str>).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_webui_arguments, parse_webui_ready_url};

    #[test]
    fn webui_ready_url_uses_reported_port() {
        assert_eq!(parse_webui_ready_url("[WebUI] Running on local URL:  http://127.0.0.1:7861\r"), Some("http://127.0.0.1:7861".to_owned()));
        assert_eq!(parse_webui_ready_url("Running on local URL: http://127.0.0.1:0"), None);
        assert_eq!(parse_webui_ready_url("Running on local URL: http://127.0.0.1:99999"), None);
        assert_eq!(parse_webui_ready_url("Running on public URL: http://127.0.0.1:7860"), None);
    }

    #[test]
    fn webui_arguments_reject_duplicate_flags() {
        assert!(parse_webui_arguments(&["--lowvram".to_owned(), "--lowvram".to_owned()]).is_err());
        assert_eq!(parse_webui_arguments(&["--medvram".to_owned(), "--xformers".to_owned()]).unwrap(), vec!["--medvram", "--xformers"]);
    }
}
