use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, process::{Command, Stdio}};
use tauri::AppHandle;

use crate::installer::{hide_console, InstallControl};

const GIT_URL: &str = "https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.3/MinGit-2.56.0.3-64-bit.zip";
const GIT_SHA256: &str = "2e5619966e588fde5f8122c5557cad50c0fbafcaed736689f40ec1dfb1d78ea7";
const UV_URL: &str = "https://github.com/astral-sh/uv/releases/download/0.13.5/uv-x86_64-pc-windows-msvc.zip";
const UV_SHA256: &str = "a38b8d1902b7cfbc76b423ba1bb456f623f42064cdc72628b6075b89afd1019a";

pub(crate) fn git_path(parent: &Path) -> PathBuf {
    parent.join(".flow-tools").join("mingit").join("cmd").join("git.exe")
}

pub(crate) fn git_program(app: &AppHandle) -> PathBuf {
    crate::installation_dir(app).ok().map(|parent| git_path(&parent))
        .filter(|path| path.is_file()).unwrap_or_else(|| PathBuf::from("git"))
}

pub(crate) fn add_git_to_path(command: &mut Command, parent: &Path) {
    let git_cmd = git_path(parent).parent().map(Path::to_path_buf);
    if let Some(git_cmd) = git_cmd.filter(|path| path.is_dir()) {
        let mut paths = vec![git_cmd];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        if let Ok(joined) = std::env::join_paths(paths) { command.env("PATH", joined); }
    }
}

fn available(program: &Path, args: &[&str]) -> bool {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_console(&mut command);
    command.status().is_ok_and(|status| status.success())
}

fn checked_archive(url: &str, expected: &str, destination: &Path, control: &InstallControl) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Flow/1.1 managed-tools")
        .timeout(std::time::Duration::from_secs(600))
        .build().map_err(|error| error.to_string())?;
    let mut response = client.get(url).send().and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Не удалось скачать инструмент: {error}"))?;
    let parent = destination.parent().ok_or("Некорректная папка инструментов.")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?.as_nanos();
    let staging = parent.join(format!(".download-{}-{stamp}", std::process::id()));
    if staging.exists() { return Err("Временная папка инструмента занята. Повторите установку позже.".to_owned()); }
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    let result = (|| {
        let archive_path = staging.join("tool.zip");
        let mut archive_file = fs::File::create(&archive_path).map_err(|error| error.to_string())?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if control.is_cancelled() { return Err("Установка отменена.".to_owned()); }
            let count = response.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 { break; }
            archive_file.write_all(&buffer[..count]).map_err(|error| error.to_string())?;
            digest.update(&buffer[..count]);
        }
        archive_file.flush().map_err(|error| error.to_string())?;
        if format!("{:x}", digest.finalize()) != expected {
            return Err("Контрольная сумма загруженного инструмента не совпадает; установка остановлена.".to_owned());
        }
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        let unpacked = staging.join("unpacked");
        fs::create_dir(&unpacked).map_err(|error| error.to_string())?;
        for index in 0..zip.len() {
            if control.is_cancelled() { return Err("Установка отменена.".to_owned()); }
            let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
            let relative = entry.enclosed_name().ok_or("Недопустимый путь в архиве инструмента.")?;
            let path = unpacked.join(relative);
            if entry.is_dir() { fs::create_dir_all(&path).map_err(|error| error.to_string())?; continue; }
            if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
            let mut output = fs::File::create(&path).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
        }
        if destination.exists() { return Err("Папка встроенного инструмента уже занята. Проверьте .flow-tools.".to_owned()); }
        fs::rename(&unpacked, destination).map_err(|error| error.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

pub(crate) fn ensure_git(_app: &AppHandle, control: &InstallControl, parent: &Path) -> Result<PathBuf, String> {
    let system = PathBuf::from("git");
    if available(&system, &["--version"]) { return Ok(system); }
    if !cfg!(windows) { return Err("Git не найден. Установите Git для этой платформы.".to_owned()); }
    let managed = git_path(parent);
    if available(&managed, &["--version"]) { return Ok(managed); }
    checked_archive(GIT_URL, GIT_SHA256, &parent.join(".flow-tools/mingit"), control)?;
    if !available(&managed, &["--version"]) { return Err("Встроенный Git не запустился после установки.".to_owned()); }
    Ok(managed)
}

pub(crate) fn ensure_uv(control: &InstallControl, parent: &Path) -> Result<PathBuf, String> {
    let system = PathBuf::from("uv");
    if available(&system, &["--version"]) { return Ok(system); }
    if !cfg!(windows) { return Err("Нужен uv для подготовки Python 3.13.".to_owned()); }
    let directory = parent.join(".flow-tools/uv");
    let managed = directory.join("uv.exe");
    if available(&managed, &["--version"]) { return Ok(managed); }
    checked_archive(UV_URL, UV_SHA256, &directory, control)?;
    if !available(&managed, &["--version"]) { return Err("Встроенный uv не запустился после установки.".to_owned()); }
    Ok(managed)
}
