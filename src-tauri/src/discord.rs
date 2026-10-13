#[cfg(windows)]
mod windows {
    use serde_json::json;
    use std::{fs::{File, OpenOptions}, io::Write, sync::{Mutex, OnceLock}, time::{SystemTime, UNIX_EPOCH}};

    const CLIENT_ID: &str = "1559517880825942036";
    static CONNECTION: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    static STARTED: OnceLock<u64> = OnceLock::new();

    fn send_frame(pipe: &mut File, opcode: u32, payload: serde_json::Value) -> std::io::Result<()> {
        let data = payload.to_string();
        pipe.write_all(&opcode.to_le_bytes())?;
        pipe.write_all(&(data.len() as u32).to_le_bytes())?;
        pipe.write_all(data.as_bytes())
    }

    fn connect() -> Option<File> {
        for index in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{index}");
            if let Ok(mut pipe) = OpenOptions::new().read(true).write(true).open(path) {
                if send_frame(&mut pipe, 0, json!({"v": 1, "client_id": CLIENT_ID})).is_ok() { return Some(pipe); }
            }
        }
        None
    }

    pub(super) fn set_page(page: &str) {
        let page = page.to_owned();
        std::thread::spawn(move || {
            let state = match page.as_str() {
                "home" => "Главная", "gallery" => "Просмотр галереи",
                "models" => "Загрузка моделей", "settings" => "В настройках", _ => return,
            };
            let started = *STARTED.get_or_init(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs());
            let connection = CONNECTION.get_or_init(|| Mutex::new(None));
            let Ok(mut slot) = connection.lock() else { return };
            if slot.is_none() { *slot = connect(); }
            let Some(pipe) = slot.as_mut() else { return };
            let mut activity = json!({"state": state, "timestamps": {"start": started}});
            if page == "settings" { activity["details"] = json!("Управление Flow"); }
            let payload = json!({"cmd": "SET_ACTIVITY", "args": {"pid": std::process::id(), "activity": activity}, "nonce": format!("{started}-{page}")});
            if send_frame(pipe, 1, payload).is_err() { *slot = None; }
        });
    }
}

pub(crate) fn set_page(page: &str) {
    #[cfg(windows)] windows::set_page(page);
    #[cfg(not(windows))] let _ = page;
}
