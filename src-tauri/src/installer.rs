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
const FORGE_NEO_REPO: &str = "https://github.com/Haoming02/sd-webui-forge-classic.git";
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
    pub(crate) fn try_start(&self) -> Result<(), String> {
        if self.running.swap(true, Ordering::SeqCst) { return Err("Установка или обслуживание уже выполняется.".to_owned()); }
        self.cancelled.store(false, Ordering::SeqCst);
        Ok(())
    }
    pub(crate) fn finish(&self) { self.running.store(false, Ordering::SeqCst); }
    pub(crate) fn was_cancelled(&self) -> bool { self.cancelled.load(Ordering::SeqCst) }
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
    package_id: Arc<Mutex<Option<String>>>,
    #[cfg(windows)]
    job: Arc<Mutex<Option<WindowsJob>>>,
}

impl WebuiControl {
    pub(crate) fn package_id(&self) -> Option<String> {
        self.package_id.lock().unwrap_or_else(|poison| poison.into_inner()).clone()
    }
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

pub(crate) fn venv_python(directory: &Path) -> PathBuf {
    python_in_venv(&directory.join("venv"))
}

fn python_in_venv(venv: &Path) -> PathBuf {
    if cfg!(windows) { venv.join("Scripts/python.exe") } else { venv.join("bin/python") }
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

pub(crate) fn hide_console(command: &mut Command) {
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

pub(crate) fn command_output(app: &AppHandle, program: &str, args: &[String]) -> Result<String, String> {
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

fn uv_for_forge(app: &AppHandle, control: &InstallControl, parent: &Path) -> Result<PathBuf, String> {
    let mut probe = Command::new("uv");
    probe.arg("--version").stdin(Stdio::null());
    hide_console(&mut probe);
    if probe.output().is_ok_and(|output| output.status.success()) {
        notify(app, "checking", "uv найден; он подготовит Python 3.13 для Forge Neo.");
        return Ok(PathBuf::from("uv"));
    }

    // A small, separate bootstrap environment lets uv download Python 3.13
    // without changing the user's system Python or Stability Matrix package.
    let tools_dir = parent.join(".flow-tools");
    let bootstrap = tools_dir.join("uv-bootstrap");
    let uv = if cfg!(windows) { bootstrap.join("Scripts/uv.exe") } else { bootstrap.join("bin/uv") };
    if uv.is_file() { return Ok(uv); }
    notify(app, "checking", "uv не найден; устанавливаю его для подготовки Python 3.13…");
    let (python, mut prefix) = python_310(app).map_err(|_| "Для автоматической установки Python 3.13 нужен uv или Python 3.10. Установите uv и повторите установку Forge Neo.".to_owned())?;
    fs::create_dir_all(&tools_dir).map_err(|error| error.to_string())?;
    prefix.extend(["-m".to_owned(), "venv".to_owned(), bootstrap.to_string_lossy().into_owned()]);
    let bootstrap_python = python_in_venv(&bootstrap);
    if !bootstrap_python.is_file() {
        run_logged(app, control, &python, &prefix, &tools_dir, None)?;
    }
    run_logged_retry_locked(app, control, &bootstrap_python.to_string_lossy(),
        &["-m".to_owned(), "pip".to_owned(), "install".to_owned(), "--upgrade".to_owned(), "uv".to_owned()], &tools_dir)?;
    if !uv.is_file() { return Err("uv не появился после установки.".to_owned()); }
    Ok(uv)
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
    let port_text = ["http://127.0.0.1:", "http://localhost:"]
        .iter().find_map(|host| suffix.split_once(host).map(|(_, port)| port))?;
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

fn emit_webui_line(app: &AppHandle, control: &WebuiControl, line: &[u8], source: &'static str) {
    if source == "stderr" {
        let mut tagged = b"[stderr] ".to_vec();
        tagged.extend_from_slice(line);
        emit_pipe_line(app, "launch", "log", &tagged, None, Some(control));
    } else {
        emit_pipe_line(app, "launch", "log", line, None, Some(control));
    }
    if source == "stderr" && is_windows_commit_error(&String::from_utf8_lossy(line)) {
        notify_kind(app, "launch", "model_error", "Windows не смогла выделить память для загрузки модели (ошибка 1455). WebUI продолжает работать, но эта модель не загрузилась. Проверьте «Выделено» в Диспетчере задач → Производительность → Память; при достижении лимита увеличьте файл подкачки Windows и убедитесь, что на диске есть свободное место.");
    }
}

fn is_windows_commit_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("os error 1455") || lower.contains("winerror 1455")
}

fn tail_webui_log(app: AppHandle, mut reader: fs::File, control: WebuiControl, source: &'static str) -> thread::JoinHandle<()> {
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
                                emit_webui_line(&app, &control, &line, source);
                                line.clear();
                            }
                        } else {
                            line.push(byte);
                            if line.len() >= 16_384 {
                            emit_webui_line(&app, &control, &line, source);
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
        if !line.is_empty() { emit_webui_line(&app, &control, &line, source); }
    })
}

fn run_logged_kind(app: &AppHandle, control: &InstallControl, kind: &'static str, program: &str, args: &[String], cwd: &Path, locked_file: Option<Arc<AtomicBool>>, env: &[(&str, &Path)]) -> Result<(), String> {
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd)
        .env("PYTHONUNBUFFERED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let requirements = cwd.join(".flow-requirements.txt");
    if requirements.is_file() { command.env("REQS_FILE", requirements); }
    for &(key, value) in env { command.env(key, value); }
    use_local_clip_sources(&mut command, cwd);
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|error| format!("Не удалось запустить {program}: {error}"))?;
    if let Ok(mut pid) = control.pid.lock() { *pid = Some(child.id()); }
    let stdout = child.stdout.take().map(|pipe| pipe_logs(app.clone(), kind, "log", pipe, locked_file.clone(), None));
    let stderr = child.stderr.take().map(|pipe| pipe_logs(app.clone(), kind, "log", pipe, locked_file, None));
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

pub(crate) fn run_logged(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path, locked_file: Option<Arc<AtomicBool>>) -> Result<(), String> {
    run_logged_kind(app, control, "install", program, args, cwd, locked_file, &[])
}

fn run_logged_with_env(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path, env: &[(&str, &Path)]) -> Result<(), String> {
    run_logged_kind(app, control, "install", program, args, cwd, None, env)
}

const SDXL_ALLOC_CONF: &str = "backend:native,garbage_collection_threshold:0.6,max_split_size_mb:128";
const FLOW_SAFE_SWITCH: &str = include_str!("../resources/flow_safe_switch.py");
const FLOW_SAFETENSORS: &str = "safetensors==0.8.0";

fn flow_requirements_content(source: &str) -> String {
    let mut lines = Vec::new();
    let mut replaced = false;
    for line in source.lines() {
        if line.trim_start().starts_with("safetensors==") {
            if !replaced { lines.push(FLOW_SAFETENSORS.to_owned()); }
            replaced = true;
        } else {
            lines.push(line.to_owned());
        }
    }
    if !replaced { lines.push(FLOW_SAFETENSORS.to_owned()); }
    format!("{}\n", lines.join("\n"))
}

fn flow_requirements(directory: &Path) -> Result<PathBuf, String> {
    let source = fs::read_to_string(directory.join("requirements_versions.txt"))
        .map_err(|error| format!("Не удалось прочитать зависимости WebUI: {error}"))?;
    let content = flow_requirements_content(&source);
    let destination = directory.join(".flow-requirements.txt");
    if fs::read_to_string(&destination).ok().as_deref() != Some(content.as_str()) {
        fs::write(&destination, content)
            .map_err(|error| format!("Не удалось создать зависимости Flow: {error}"))?;
    }
    Ok(destination)
}

fn ensure_flow_safe_switch_extension(directory: &Path) -> Result<(), String> {
    let script = directory.join("extensions/flow-safe-switch/scripts/flow_safe_switch.py");
    fs::create_dir_all(script.parent().expect("extension script has a parent"))
        .map_err(|error| format!("Не удалось создать расширение очистки памяти: {error}"))?;
    fs::write(&script, FLOW_SAFE_SWITCH)
        .map_err(|error| format!("Не удалось обновить расширение очистки памяти: {error}"))
}

fn nvidia_vram_mib() -> Option<u64> {
    let mut candidates = vec![PathBuf::from("nvidia-smi")];
    if cfg!(windows) {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(program_files).join("NVIDIA Corporation/NVSMI/nvidia-smi.exe"));
        }
    }
    for program in candidates {
        let mut command = Command::new(program);
        command.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .stdin(Stdio::null());
        hide_console(&mut command);
        if let Ok(output) = command.output() {
            if output.status.success() {
                if let Some(mib) = String::from_utf8_lossy(&output.stdout).lines().next().and_then(|line| line.trim().parse().ok()) {
                    return Some(mib);
                }
            }
        }
    }
    None
}

fn apply_memory_flags(arguments: &mut Vec<String>, memory_profile: bool, vram_mib: Option<u64>) -> bool {
    let low_vram = vram_mib.is_some_and(|mib| mib <= 5_120);
    let constrained_vram = vram_mib.is_some_and(|mib| mib <= 6_144);
    if !memory_profile && !constrained_vram { return false; }
    let has_vram_flag = arguments.iter().any(|argument| matches!(argument.as_str(), "--lowvram" | "--medvram" | "--medvram-sdxl"));
    let has_attention_flag = arguments.iter().any(|argument| matches!(argument.as_str(), "--xformers" | "--opt-sub-quad-attention" | "--opt-sdp-attention" | "--opt-sdp-no-mem-attention"));
    if !has_vram_flag {
        arguments.push(if low_vram { "--lowvram" } else { "--medvram-sdxl" }.to_owned());
    }
    if !has_attention_flag {
        arguments.push(if low_vram { "--opt-sdp-no-mem-attention" } else { "--opt-sub-quad-attention" }.to_owned());
    }
    true
}

fn update_sdxl_memory_config(config: &mut serde_json::Value) -> Result<(), String> {
    let options = config.as_object_mut().ok_or("config.json WebUI должен содержать объект JSON.")?;
    // A1111 keeps extra checkpoints in RAM/VRAM when this limit is higher than one.
    options.insert("sd_checkpoints_limit".to_owned(), json!(1));
    options.insert("sd_checkpoints_keep_in_cpu".to_owned(), json!(true));
    options.insert("sd_checkpoint_cache".to_owned(), json!(0));
    options.insert("sd_vae_checkpoint_cache".to_owned(), json!(0));
    // A1111's disable-mmap branch reads the whole checkpoint into one buffer
    // and fails with MemoryError on low-RAM systems. The Flow extension uses
    // safetensors 0.8's streaming pread backend instead.
    if cfg!(windows) {
        options.insert("disable_mmap_load_safetensors".to_owned(), json!(false));
    }
    Ok(())
}

fn apply_sdxl_memory_settings(directory: &Path) -> Result<(), String> {
    let path = directory.join("config.json");
    let mut config = if path.is_file() {
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Некорректный config.json WebUI: {error}"))?
    } else { json!({}) };
    update_sdxl_memory_config(&mut config)?;
    fs::write(&path, serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Не удалось сохранить настройки памяти WebUI: {error}"))
}

fn run_logged_retry_locked_env(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path, env: &[(&str, &Path)]) -> Result<(), String> {
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        let locked_file = Arc::new(AtomicBool::new(false));
        match run_logged_kind(app, control, "install", program, args, cwd, Some(Arc::clone(&locked_file)), env) {
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

fn run_logged_retry_locked(app: &AppHandle, control: &InstallControl, program: &str, args: &[String], cwd: &Path) -> Result<(), String> {
    run_logged_retry_locked_env(app, control, program, args, cwd, &[])
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

fn only_preloaded_models(directory: &Path) -> Result<bool, String> {
    let entries = fs::read_dir(directory).map_err(|error| error.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    if entries.len() != 1 || entries[0].file_name().to_string_lossy() != "models" || !entries[0].file_type().map_err(|error| error.to_string())?.is_dir() { return Ok(false); }
    let models = fs::read_dir(directory.join("models")).map_err(|error| error.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    if models.len() != 1 || models[0].file_name().to_string_lossy() != "Stable-diffusion" || !models[0].file_type().map_err(|error| error.to_string())?.is_dir() { return Ok(false); }
    for entry in fs::read_dir(directory.join("models/Stable-diffusion")).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !entry.file_type().map_err(|error| error.to_string())?.is_file()
            || !(name.ends_with(".safetensors") || name.ends_with(".ckpt") || name.ends_with(".flow-part") || name.ends_with(".flow-meta.json")) { return Ok(false); }
    }
    Ok(true)
}

pub(crate) fn install(app: &AppHandle, control: &InstallControl, parent: &Path) -> Result<(), String> {
    notify(app, "checking", "Проверка Git и Python 3.10…");
    command_output(app, "git", &["--version".to_owned()]).map_err(|_| "Git не найден. Установите Git и перезапустите Flow.".to_owned())?;
    let (python, prefix) = python_310(app)?;
    let directory = parent.join("sd-webui");
    notify(app, "cloning", format!("Папка пакета: {}", directory.display()));
    if directory.exists() {
        if !directory.join(".git").is_dir() {
            let empty = fs::read_dir(&directory).map_err(|error| error.to_string())?.next().is_none();
            if !empty && !only_preloaded_models(&directory)? { return Err("Папка sd-webui занята другим содержимым.".to_owned()); }
            notify(app, "cloning", "Продолжаю подготовку репозитория WebUI.");
            run_logged(app, control, "git", &["init".to_owned()], &directory, None)?;
            run_logged(app, control, "git", &["remote".to_owned(), "add".to_owned(), "origin".to_owned(), WEBUI_REPO.to_owned()], &directory, None)?;
        }
        let remote = command_output(app, "git", &["-C".to_owned(), directory.to_string_lossy().into_owned(), "remote".to_owned(), "get-url".to_owned(), "origin".to_owned()])?;
        if remote.trim_end_matches('/').trim_end_matches(".git").to_lowercase() != WEBUI_REPO.trim_end_matches(".git").to_lowercase() {
            return Err("В выбранной папке находится другой Git-репозиторий.".to_owned());
        }
        notify(app, "cloning", "Найден существующий репозиторий WebUI; продолжаю установку.");
        if !directory.join("launch.py").is_file() {
            run_logged(app, control, "git", &["fetch".to_owned(), "--depth".to_owned(), "1".to_owned(), "origin".to_owned(), "HEAD".to_owned()], &directory, None)?;
            run_logged(app, control, "git", &["checkout".to_owned(), "--detach".to_owned(), "FETCH_HEAD".to_owned()], &directory, None)?;
        }
    } else {
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        run_logged(app, control, "git", &["init".to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["remote".to_owned(), "add".to_owned(), "origin".to_owned(), WEBUI_REPO.to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["fetch".to_owned(), "--progress".to_owned(), "--depth".to_owned(), "1".to_owned(), "origin".to_owned(), "HEAD".to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["checkout".to_owned(), "--detach".to_owned(), "FETCH_HEAD".to_owned()], &directory, None)?;
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
    flow_requirements(&directory)?;
    // Keep WebUI's dependency pins except for safetensors, which needs the
    // streaming pread backend in Flow's low-memory Windows profile.
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

pub(crate) fn install_forge_neo(app: &AppHandle, control: &InstallControl, parent: &Path) -> Result<(), String> {
    notify(app, "checking", "Проверка Git для Forge Neo…");
    command_output(app, "git", &["--version".to_owned()])
        .map_err(|_| "Git не найден. Установите Git и перезапустите Flow.".to_owned())?;
    let directory = parent.join("sd-webui-forge-neo");
    notify(app, "cloning", format!("Forge Neo будет установлен отдельно: {}", directory.display()));
    if directory.exists() {
        if !directory.join(".git").is_dir() {
            if fs::read_dir(&directory).map_err(|error| error.to_string())?.next().is_some() {
                return Err("Папка sd-webui-forge-neo уже занята. Выберите другую папку установки или освободите её.".to_owned());
            }
            run_logged(app, control, "git", &["init".to_owned()], &directory, None)?;
            run_logged(app, control, "git", &["remote".to_owned(), "add".to_owned(), "origin".to_owned(), FORGE_NEO_REPO.to_owned()], &directory, None)?;
        }
        let path = directory.to_string_lossy().into_owned();
        let remote = command_output(app, "git", &["-C".to_owned(), path.clone(), "remote".to_owned(), "get-url".to_owned(), "origin".to_owned()])?;
        if remote.trim_end_matches('/').trim_end_matches(".git").to_lowercase() != FORGE_NEO_REPO.trim_end_matches(".git").to_lowercase() {
            return Err("В папке sd-webui-forge-neo находится другой Git-репозиторий.".to_owned());
        }
        let branch = command_output(app, "git", &["-C".to_owned(), path.clone(), "branch".to_owned(), "--show-current".to_owned()])?;
        if branch != "neo" && !branch.is_empty() { return Err("Существующий Forge-репозиторий находится не на ветке neo.".to_owned()); }
        if !directory.join("launch.py").is_file() || !directory.join("modules_forge").is_dir() {
            let changes = command_output(app, "git", &["-C".to_owned(), path, "status".to_owned(), "--porcelain".to_owned()])?;
            if !changes.is_empty() { return Err("В неполной папке Forge Neo есть локальные изменения. Сохраните их перед повторной установкой.".to_owned()); }
            notify(app, "cloning", "Продолжаю прерванное скачивание Forge Neo…");
            run_logged(app, control, "git", &["fetch".to_owned(), "--depth".to_owned(), "1".to_owned(), "origin".to_owned(), "neo".to_owned()], &directory, None)?;
            if branch.is_empty() {
                run_logged(app, control, "git", &["checkout".to_owned(), "-B".to_owned(), "neo".to_owned(), "FETCH_HEAD".to_owned()], &directory, None)?;
            } else {
                run_logged(app, control, "git", &["reset".to_owned(), "--hard".to_owned(), "FETCH_HEAD".to_owned()], &directory, None)?;
            }
        }
        notify(app, "cloning", "Найден ранее скачанный Forge Neo; продолжаю установку.");
    } else {
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        run_logged(app, control, "git", &["init".to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["remote".to_owned(), "add".to_owned(), "origin".to_owned(), FORGE_NEO_REPO.to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["fetch".to_owned(), "--progress".to_owned(), "--depth".to_owned(), "1".to_owned(), "origin".to_owned(), "neo".to_owned()], &directory, None)?;
        run_logged(app, control, "git", &["checkout".to_owned(), "-B".to_owned(), "neo".to_owned(), "FETCH_HEAD".to_owned()], &directory, None)?;
    }
    if !directory.join("launch.py").is_file() || !directory.join("modules_forge").is_dir() {
        return Err("В скачанном репозитории Forge Neo не найдены файлы запуска.".to_owned());
    }

    let uv = uv_for_forge(app, control, parent)?;
    let tools_dir = parent.join(".flow-tools");
    let managed_python = tools_dir.join("python");
    let uv_cache = tools_dir.join("uv-cache");
    let pip_cache = tools_dir.join("pip-cache");
    let python = venv_python(&directory);
    if !python.is_file() {
        notify(app, "venv", "Создание отдельного окружения Python 3.13 для Forge Neo…");
        run_logged_with_env(app, control, &uv.to_string_lossy(), &[
            "venv".to_owned(), "venv".to_owned(), "--python".to_owned(), "3.13".to_owned(), "--seed".to_owned(),
        ], &directory, &[("UV_PYTHON_INSTALL_DIR", managed_python.as_path()), ("UV_CACHE_DIR", uv_cache.as_path())])?;
    }
    if !python.is_file() { return Err("Окружение Python Forge Neo не было создано.".to_owned()); }
    let version = command_output(app, &python.to_string_lossy(), &[
        "-c".to_owned(), "import sys; print('%d.%d' % sys.version_info[:2])".to_owned(),
    ])?;
    if version != "3.13" {
        return Err(format!("Forge Neo требует Python 3.13, а найден {version}. Удалите только папку venv внутри sd-webui-forge-neo и повторите установку."));
    }
    notify(app, "dependencies", "Обновление pip в окружении Forge Neo…");
    run_logged_retry_locked_env(app, control, &python.to_string_lossy(), &[
        "-m".to_owned(), "pip".to_owned(), "install".to_owned(), "--upgrade".to_owned(), "pip".to_owned(),
    ], &directory, &[("PIP_CACHE_DIR", pip_cache.as_path())])?;
    notify(app, "dependencies", "Forge Neo устанавливает PyTorch и остальные зависимости; это может занять некоторое время…");
    run_logged_retry_locked_env(app, control, &python.to_string_lossy(), &[
        "-u".to_owned(), "launch.py".to_owned(), "--exit".to_owned(),
    ], &directory, &[("PIP_CACHE_DIR", pip_cache.as_path())])?;
    if control.cancelled.load(Ordering::SeqCst) { return Err("Установка отменена.".to_owned()); }
    if !python_module_installed(&python, "torch")? || !python_module_installed(&python, "gradio")? {
        return Err("Зависимости Forge Neo установились не полностью. Повторите установку.".to_owned());
    }
    let version = command_output(app, "git", &[
        "-C".to_owned(), directory.to_string_lossy().into_owned(), "describe".to_owned(),
        "--tags".to_owned(), "--always".to_owned(),
    ]).unwrap_or_else(|_| "Установлен".to_owned());
    fs::write(directory.join(".flow-installed.json"), json!({ "name": "Forge Neo", "version": version }).to_string())
        .map_err(|error| error.to_string())?;
    crate::activate_managed_forge_neo(app, &directory)?;
    notify(app, "done", "Forge Neo установлен отдельно и готов к запуску.");
    Ok(())
}

#[tauri::command]
pub(crate) async fn install_forge_neo_web_ui(app: AppHandle, state: State<'_, Arc<InstallControl>>, webui: State<'_, WebuiControl>, downloads: State<'_, Arc<crate::civitai::DownloadControl>>, installation_dir: String) -> Result<(), String> {
    if webui.is_running() { return Err("Остановите WebUI перед установкой Forge Neo.".to_owned()); }
    if downloads.is_running() { return Err("Дождитесь завершения скачивания модели перед установкой Forge Neo.".to_owned()); }
    let selected = crate::paths::subprocess_path(&PathBuf::from(installation_dir.trim()));
    if !selected.is_absolute() || !selected.is_dir() { return Err("Выберите существующую папку установки.".to_owned()); }
    let selected = crate::paths::subprocess_path(&selected.canonicalize().map_err(|error| error.to_string())?);
    let saved = crate::paths::subprocess_path(&crate::installation_dir(&app)?.canonicalize().map_err(|error| error.to_string())?);
    if selected != saved { return Err("Папка установки изменилась. Повторите выбор.".to_owned()); }
    if state.running.swap(true, Ordering::SeqCst) { return Err("Установка уже выполняется.".to_owned()); }
    state.cancelled.store(false, Ordering::SeqCst);
    let control = Arc::clone(&*state);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = install_forge_neo(&app, &control, &selected) {
            notify(&app, if control.cancelled.load(Ordering::SeqCst) { "cancelled" } else { "error" }, error);
        }
        control.running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub(crate) async fn install_web_ui(app: AppHandle, state: State<'_, Arc<InstallControl>>, webui: State<'_, WebuiControl>, downloads: State<'_, Arc<crate::civitai::DownloadControl>>, installation_dir: String) -> Result<(), String> {
    if webui.is_running() { return Err("Остановите WebUI перед установкой пакета.".to_owned()); }
    if downloads.is_running() { return Err("Дождитесь завершения скачивания модели перед установкой WebUI.".to_owned()); }
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

fn installed_webui_python(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let directory = installation_dir(app)?.join("sd-webui");
    if !directory.join(".flow-installed.json").is_file() {
        return Err("Сначала установите WebUI.".to_owned());
    }
    if !directory.join("launch.py").is_file() { return Err("Файл запуска WebUI не найден.".to_owned()); }
    let python = if cfg!(windows) { directory.join("venv/Scripts/python.exe") } else { directory.join("venv/bin/python") };
    if !python.is_file() { return Err("Виртуальное окружение WebUI не найдено.".to_owned()); }
    Ok((directory, python))
}

fn xformers_version(directory: &Path, python: &Path) -> Result<String, String> {
    let mut command = Command::new(python);
    command.args(["-c", "import xformers, xformers.ops; print(xformers.__version__)"])
        .current_dir(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut command);
    let output = command.output().map_err(|error| format!("Не удалось проверить xformers: {error}"))?;
    if !output.status.success() {
        return Err("xformers не установлен или не загружается в окружении WebUI.".to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[tauri::command]
pub(crate) async fn check_xformers(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (directory, python) = installed_webui_python(&app)?;
        xformers_version(&directory, &python)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn install_xformers(
    app: AppHandle,
    state: State<'_, Arc<InstallControl>>,
    webui: State<'_, WebuiControl>,
) -> Result<(), String> {
    if webui.is_running() { return Err("Остановите WebUI перед установкой xformers.".to_owned()); }
    let (directory, python) = installed_webui_python(&app)?;
    if state.running.swap(true, Ordering::SeqCst) { return Err("Уже выполняется установка.".to_owned()); }
    state.cancelled.store(false, Ordering::SeqCst);
    let control = Arc::clone(&*state);
    tauri::async_runtime::spawn_blocking(move || {
        notify_kind(&app, "xformers", "dependencies", "Проверка xformers в окружении WebUI…");
        let result = if xformers_version(&directory, &python).is_ok() {
            Ok(())
        } else {
            notify_kind(&app, "xformers", "dependencies", "WebUI устанавливает совместимую версию xformers…");
            // Let the WebUI launcher choose its pinned wheel and use --no-deps.
            run_logged_kind(&app, &control, "xformers", &python.to_string_lossy(),
                &["-u".to_owned(), "launch.py".to_owned(), "--exit".to_owned(), "--xformers".to_owned(), "--reinstall-xformers".to_owned(), "--skip-torch-cuda-test".to_owned()],
                &directory, None, &[])
                .and_then(|_| xformers_version(&directory, &python).map(|_| ()))
        };
        match result {
            Ok(()) => notify_kind(&app, "xformers", "done", "xformers доступен в окружении WebUI. Для использования включите --xformers и перезапустите WebUI."),
            Err(error) => notify_kind(&app, "xformers", if control.cancelled.load(Ordering::SeqCst) { "cancelled" } else { "error" }, error),
        }
        control.running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn launch_web_ui(app: AppHandle, webui: State<'_, WebuiControl>, installs: State<'_, Arc<InstallControl>>, package_id: Option<String>) -> Result<(), String> {
    if webui.is_running() { return Err("WebUI уже запущен или запускается.".to_owned()); }
    if installs.is_running() { return Err("Дождитесь завершения установки пакета или xformers.".to_owned()); }
    let package_id = package_id.unwrap_or(crate::active_package(&app)?);
    if package_id != "automatic1111" && package_id != "forge-neo" { return Err("Неизвестный пакет.".to_owned()); }
    if crate::active_package(&app)? != package_id { return Err("Сначала выберите пакет в списке Flow.".to_owned()); }
    let forge = package_id == "forge-neo";
    let directory = if forge {
        crate::package_dir(&app)?
    } else { installation_dir(&app)?.join("sd-webui") };
    if !forge && !directory.join(".flow-installed.json").is_file() { return Err("Сначала установите пакет.".to_owned()); }
    if !directory.join("launch.py").is_file() { return Err("Файл запуска WebUI не найден.".to_owned()); }
    if forge && !directory.join("modules_forge").is_dir() { return Err("Выбрана не папка Forge Neo.".to_owned()); }
    let requirements = if forge { None } else { Some(flow_requirements(&directory)?) };
    let python = venv_python(&directory);
    if !python.is_file() { return Err("Виртуальное окружение WebUI не найдено.".to_owned()); }
    let mut arguments = parse_webui_arguments(&crate::webui_arguments(&app)?)?;
    let library = crate::models_library_dir(&app)?;
    let checkpoints = library.join("Stable-diffusion");
    fs::create_dir_all(&checkpoints).map_err(|error| format!("Не удалось подготовить общую библиотеку моделей: {error}"))?;
    if forge {
        // An imported Forge may already have its own model layout. Keep that layout intact.
        let managed = crate::installation_dir(&app)?.join("sd-webui-forge-neo");
        if directory == managed && !arguments.iter().any(|arg| arg == "--model-ref" || arg.starts_with("--model-ref=")) {
            let (linked, conflicts) = crate::maintenance::link_model_tree(&directory.join("models"), &library)?;
            if linked > 0 { notify_kind(&app, "launch", "log", format!("В общую библиотеку без копирования добавлено файлов: {linked}.")); }
            if conflicts > 0 { notify_kind(&app, "launch", "log", format!("В общей библиотеке уже есть {conflicts} файлов с теми же именами. Проверьте их при необходимости.")); }
            arguments.push("--model-ref".to_owned());
            arguments.push(library.to_string_lossy().into_owned());
        }
    } else if !arguments.iter().any(|arg| arg == "--ckpt-dir" || arg.starts_with("--ckpt-dir=")) {
        arguments.push("--ckpt-dir".to_owned());
        arguments.push(checkpoints.to_string_lossy().into_owned());
    }
    let detected_vram = nvidia_vram_mib();
    let memory_profile = !forge && apply_memory_flags(&mut arguments, crate::sdxl_memory_profile(&app)?, detected_vram);
    if memory_profile {
        if arguments.iter().any(|arg| matches!(arg.as_str(), "--disable-extra-extensions" | "--disable-all-extensions")) {
            return Err("Режим безопасной смены моделей требует расширение Flow. Уберите --disable-extra-extensions/--disable-all-extensions из аргументов запуска.".to_owned());
        }
        apply_sdxl_memory_settings(&directory)?;
        ensure_flow_safe_switch_extension(&directory)?;
    }
    // webui-user.bat clears COMMANDLINE_ARGS and does not forward its own arguments.
    // Passing args directly to launch.py also avoids shell quoting and a visible CMD window.
    let mut command = Command::new(python);
    command.arg("-u").arg("launch.py").args(&arguments);
    if !forge { use_local_clip_sources(&mut command, &directory); }
    command.current_dir(&directory)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8:backslashreplace")
        .env("PYTHONFAULTHANDLER", "1")
        .env("WEBUI_LAUNCH_LIVE_OUTPUT", "1")
        .env("STABLE_DIFFUSION_REPO", SD_REPO)
        // The saved flags are passed as argv; inherited COMMANDLINE_ARGS must not repeat them.
        .env_remove("COMMANDLINE_ARGS")
        .stdin(Stdio::null());
    if let Some(requirements) = requirements { command.env("REQS_FILE", requirements); }
    else { command.env_remove("REQS_FILE").env_remove("FLOW_SAFE_MODEL_SWITCH"); }
    if memory_profile {
        // Configure only this Python process, before torch is imported.
        command.env("PYTORCH_CUDA_ALLOC_CONF", SDXL_ALLOC_CONF);
        command.env("PYTORCH_ALLOC_CONF", SDXL_ALLOC_CONF);
        command.env("FLOW_SAFE_MODEL_SWITCH", "1");
    } else {
        command.env_remove("FLOW_SAFE_MODEL_SWITCH");
    }
    hide_console(&mut command);
    if webui.running.swap(true, Ordering::SeqCst) { return Err("WebUI уже запущен или запускается.".to_owned()); }
    *webui.package_id.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(package_id);
    webui.stopping.store(false, Ordering::SeqCst);
    webui.stopped.store(false, Ordering::SeqCst);
    webui.exited.store(false, Ordering::SeqCst);
    webui.ready_pending.store(false, Ordering::SeqCst);
    webui.generation.fetch_add(1, Ordering::SeqCst);
    *webui.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    // Python writes to a real file. A WebView2/Flow crash cannot close its stdout pipe.
    let log_setup = (|| -> Result<(fs::File, fs::File, fs::File, fs::File, PathBuf, PathBuf), String> {
        let log_dir = crate::data_dir(&app)?.join("logs");
        fs::create_dir_all(&log_dir).map_err(|error| error.to_string())?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let log_path = log_dir.join(format!("webui-{}-{stamp}.stdout.log", std::process::id()));
        let stderr_path = log_dir.join(format!("webui-{}-{stamp}.stderr.log", std::process::id()));
        let stdout_writer = fs::OpenOptions::new().append(true).create_new(true).open(&log_path)
            .map_err(|error| format!("Не удалось создать журнал WebUI: {error}"))?;
        let stderr_writer = fs::OpenOptions::new().append(true).create_new(true).open(&stderr_path)
            .map_err(|error| format!("Не удалось создать журнал ошибок WebUI: {error}"))?;
        let stdout_reader = fs::File::open(&log_path).map_err(|error| format!("Не удалось открыть журнал WebUI: {error}"))?;
        let stderr_reader = fs::File::open(&stderr_path).map_err(|error| format!("Не удалось открыть журнал ошибок WebUI: {error}"))?;
        Ok((stdout_writer, stderr_writer, stdout_reader, stderr_reader, log_path, stderr_path))
    })();
    let (stdout_writer, stderr_writer, stdout_reader, stderr_reader, log_path, stderr_path) = match log_setup {
        Ok(files) => files,
        Err(error) => { webui.running.store(false, Ordering::SeqCst); return Err(error); }
    };
    command.stdout(Stdio::from(stdout_writer)).stderr(Stdio::from(stderr_writer));
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
    if let Some(mib) = detected_vram {
        notify_kind(&app, "launch", "log", format!("NVIDIA VRAM: {mib} MiB; режим экономии памяти: {}.", if memory_profile { "включён" } else { "выключен" }));
    }
    if memory_profile {
        notify_kind(&app, "launch", "log", format!("Флаги памяти и attention: {}", arguments.iter().filter(|arg| arg.starts_with("--lowvram") || arg.starts_with("--medvram") || arg.starts_with("--opt-") || arg.as_str() == "--xformers").cloned().collect::<Vec<_>>().join(" ")));
        notify_kind(&app, "launch", "log", "Веса загружаются через RAM; безопасная выгрузка предыдущего чекпоинта включена.");
        #[cfg(windows)]
        notify_kind(&app, "launch", "log", "safetensors: потоковое чтение pread вместо mmap; полный буфер модели не создаётся.");
    }
    notify_kind(&app, "launch", "log", format!("Журналы WebUI: stdout — {}; stderr — {}", log_path.display(), stderr_path.display()));
    let control = (*webui).clone();
    thread::spawn(move || {
        let stdout_logger = tail_webui_log(app.clone(), stdout_reader, control.clone(), "stdout");
        let stderr_logger = tail_webui_log(app.clone(), stderr_reader, control.clone(), "stderr");
        let result = child.wait();
        control.exited.store(true, Ordering::SeqCst);
        let was_stopping = control.stopping.swap(false, Ordering::SeqCst);
        // Terminate remaining descendants before the log tailer drains the final output.
        #[cfg(windows)] {
            if let Some(job) = control.job.lock().unwrap_or_else(|poison| poison.into_inner()).take() {
                let _ = job.terminate();
            }
        }
        let _ = stdout_logger.join();
        let _ = stderr_logger.join();
        let was_ready = control.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()).is_some();
        *control.pid.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        *control.ready_url.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        control.stopped.store(true, Ordering::SeqCst);
        control.running.store(false, Ordering::SeqCst);
        match result {
            Ok(_) if was_stopping => notify_kind(&app, "launch", "done", "WebUI остановлен пользователем."),
            Ok(status) if status.success() && was_ready => notify_kind(&app, "launch", "done", "WebUI остановлен."),
            Ok(status) if status.success() => notify_kind(&app, "launch", "error", "WebUI завершился до появления локального адреса."),
            Ok(status) if status.code() == Some(-1073741819) => notify_kind(&app, "launch", "error", format!("WebUI аварийно завершился: 0xC0000005. Python faulthandler включён; traceback (если Python успел его записать) находится в {}. Если traceback отсутствует, проверьте журнал событий Windows.", stderr_path.display())),
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
    use super::{apply_memory_flags, flow_requirements_content, is_windows_commit_error, parse_webui_arguments, parse_webui_ready_url, update_sdxl_memory_config};

    #[test]
    fn recognizes_page_file_failure_without_confusing_other_errors() {
        assert!(is_windows_commit_error("OSError: The paging file is too small for this operation to complete. (os error 1455)"));
        assert!(is_windows_commit_error("[WinError 1455]"));
        assert!(!is_windows_commit_error("Windows fatal exception: access violation"));
    }

    #[test]
    fn flow_requirements_preserve_webui_pins_except_safetensors() {
        let source = "torch\nsafetensors==0.4.2\ngradio==3.41.2\n";
        assert_eq!(flow_requirements_content(source), "torch\nsafetensors==0.8.0\ngradio==3.41.2\n");
    }

    #[test]
    fn four_gib_gpu_gets_lowvram_and_attention_once() {
        let mut arguments = vec!["--api".to_owned()];
        assert!(apply_memory_flags(&mut arguments, false, Some(4096)));
        assert_eq!(arguments.iter().map(String::as_str).collect::<Vec<_>>(), ["--api", "--lowvram", "--opt-sdp-no-mem-attention"]);
        assert!(apply_memory_flags(&mut arguments, false, Some(4096)));
        assert_eq!(arguments.len(), 3);
    }

    #[test]
    fn six_gib_gpu_stages_sdxl_weights_in_ram() {
        let mut arguments = vec!["--api".to_owned()];
        assert!(apply_memory_flags(&mut arguments, false, Some(6144)));
        assert_eq!(arguments.iter().map(String::as_str).collect::<Vec<_>>(), ["--api", "--medvram-sdxl", "--opt-sub-quad-attention"]);
    }

    #[test]
    fn chosen_memory_flags_are_respected() {
        let mut arguments = vec!["--medvram-sdxl".to_owned(), "--xformers".to_owned()];
        assert!(apply_memory_flags(&mut arguments, true, None));
        assert_eq!(arguments.iter().map(String::as_str).collect::<Vec<_>>(), ["--medvram-sdxl", "--xformers"]);
        let mut no_profile = Vec::new();
        assert!(!apply_memory_flags(&mut no_profile, false, None));
        assert!(no_profile.is_empty());
    }

    #[test]
    fn webui_ready_url_uses_reported_port() {
        assert_eq!(parse_webui_ready_url("[WebUI] Running on local URL:  http://127.0.0.1:7861\r"), Some("http://127.0.0.1:7861".to_owned()));
        assert_eq!(parse_webui_ready_url("Running on local URL:  http://localhost:7862"), Some("http://127.0.0.1:7862".to_owned()));
        assert_eq!(parse_webui_ready_url("Running on local URL: http://127.0.0.1:0"), None);
        assert_eq!(parse_webui_ready_url("Running on local URL: http://127.0.0.1:99999"), None);
        assert_eq!(parse_webui_ready_url("Running on public URL: http://127.0.0.1:7860"), None);
    }

    #[test]
    fn webui_arguments_reject_duplicate_flags() {
        assert!(parse_webui_arguments(&["--lowvram".to_owned(), "--lowvram".to_owned()]).is_err());
        assert_eq!(parse_webui_arguments(&["--medvram".to_owned(), "--xformers".to_owned()]).unwrap(), vec!["--medvram", "--xformers"]);
    }

    #[test]
    fn sdxl_profile_keeps_unrelated_webui_options() {
        let mut config = serde_json::json!({ "samples_format": "png", "sd_checkpoints_limit": 3 });
        update_sdxl_memory_config(&mut config).unwrap();
        assert_eq!(config["samples_format"], "png");
        assert_eq!(config["sd_checkpoints_limit"], 1);
        assert_eq!(config["sd_checkpoint_cache"], 0);
        assert_eq!(config["sd_vae_checkpoint_cache"], 0);
        if cfg!(windows) {
            assert_eq!(config["disable_mmap_load_safetensors"], false);
        }
    }
}
