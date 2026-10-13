"""Package chooser and live installation log for Flow."""

from queue import Empty, Queue
import threading

import customtkinter as ctk

from installer import InstallationCancelled, InstallationError, WebUIInstaller, is_installed


class InstallDialog(ctk.CTkToplevel):
    def __init__(self, master, on_installed):
        super().__init__(master)
        self.title("Добавить пакет · Flow")
        self.geometry("760x560")
        self.minsize(620, 460)
        self.configure(fg_color=master.BACKGROUND)
        self.transient(master)
        self.protocol("WM_DELETE_WINDOW", self._close)
        self.on_installed = on_installed
        self.events = Queue()
        self.installer = None
        self.installing = False
        self._log_chars = 0
        self._log_visible = True

        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(4, weight=1)

        ctk.CTkLabel(
            self, text="Добавить пакет", text_color=master.TEXT,
            font=ctk.CTkFont(size=27, weight="bold"),
        ).grid(row=0, column=0, padx=28, pady=(24, 4), sticky="w")
        ctk.CTkLabel(
            self,
            text="Выберите пакет для установки в библиотеку Flow.",
            text_color=master.MUTED,
        ).grid(row=1, column=0, padx=28, pady=(0, 16), sticky="w")

        choice = ctk.CTkFrame(
            self, corner_radius=14, fg_color=master.CARD,
            border_width=1, border_color=master.ACCENT,
        )
        choice.grid(row=2, column=0, padx=28, sticky="ew")
        choice.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            choice, text="AUTOMATIC1111 WebUI", text_color=master.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, padx=18, pady=(14, 2), sticky="w")
        ctk.CTkLabel(
            choice,
            text="Stable Diffusion · Git, Python 3.10 и отдельное виртуальное окружение",
            text_color=master.MUTED,
        ).grid(row=1, column=0, padx=18, pady=(0, 14), sticky="w")

        progress_area = ctk.CTkFrame(self, fg_color="transparent")
        progress_area.grid(row=3, column=0, padx=28, pady=(14, 0), sticky="ew")
        progress_area.grid_columnconfigure(0, weight=1)
        self.stage_label = ctk.CTkLabel(
            progress_area, text="Готово к установке", text_color=master.MUTED,
            font=ctk.CTkFont(size=13, weight="bold"),
        )
        self.stage_label.grid(row=0, column=0, sticky="w")
        self.progress = ctk.CTkProgressBar(
            progress_area, height=8, progress_color=master.ACCENT,
        )
        self.progress.grid(row=1, column=0, pady=(8, 0), sticky="ew")
        self.progress.set(0)
        self.log_toggle = ctk.CTkButton(
            progress_area, text="Скрыть журнал  ▴", width=145, height=30,
            fg_color="transparent", hover_color=master.HOVER,
            text_color=master.MUTED, command=self._toggle_log,
        )
        self.log_toggle.grid(row=2, column=0, pady=(4, 0), sticky="w")

        self.log_frame = ctk.CTkFrame(self, corner_radius=14, fg_color=master.CARD)
        self.log_frame.grid(row=4, column=0, padx=28, pady=(4, 12), sticky="nsew")
        self.log_frame.grid_columnconfigure(0, weight=1)
        self.log_frame.grid_rowconfigure(1, weight=1)
        ctk.CTkLabel(
            self.log_frame, text="Журнал установки", text_color=master.TEXT,
            font=ctk.CTkFont(size=14, weight="bold"),
        ).grid(row=0, column=0, padx=16, pady=(12, 6), sticky="w")
        self.log_box = ctk.CTkTextbox(
            self.log_frame, corner_radius=8, fg_color=master.BACKGROUND,
            text_color=master.TEXT, wrap="word", state="disabled",
        )
        self.log_box.grid(row=1, column=0, padx=16, pady=(0, 16), sticky="nsew")

        controls = ctk.CTkFrame(self, fg_color="transparent")
        controls.grid(row=5, column=0, padx=28, pady=(0, 22), sticky="ew")
        controls.grid_columnconfigure(0, weight=1)
        self.status = ctk.CTkLabel(
            controls, text="Готово к установке", text_color=master.MUTED,
        )
        self.status.grid(row=0, column=0, sticky="w")
        self.close_button = ctk.CTkButton(
            controls, text="Закрыть", width=100, fg_color=master.HOVER,
            hover_color=master.BORDER, command=self._close,
        )
        self.close_button.grid(row=0, column=1, padx=(10, 0))
        self.install_button = ctk.CTkButton(
            controls, text="Установить", width=135, fg_color=master.ACCENT,
            hover_color=master.ACCENT_HOVER, command=self._start_install,
        )
        self.install_button.grid(row=0, column=2, padx=(10, 0))

        if is_installed():
            self.status.configure(text="AUTOMATIC1111 WebUI уже установлен")
            self.install_button.configure(state="disabled")
        self.after(100, self._pump_events)
        self.after(100, self.grab_set)

    def _toggle_log(self):
        self._log_visible = not self._log_visible
        if self._log_visible:
            self.log_frame.grid()
            self.grid_rowconfigure(4, weight=1)
            self.log_toggle.configure(text="Скрыть журнал  ▴")
            self.geometry(f"{self.winfo_width()}x560")
        else:
            self.log_frame.grid_remove()
            self.grid_rowconfigure(4, weight=0)
            self.log_toggle.configure(text="Показать журнал  ▾")
            self.geometry(f"{self.winfo_width()}x460")

    def _append_log(self, message):
        self.log_box.configure(state="normal")
        self.log_box.insert("end", message + "\n")
        self._log_chars += len(message) + 1
        if self._log_chars > 120_000:
            self.log_box.delete("1.0", "1.0 + 50_000 chars")
            self._log_chars -= 50_000
        self.log_box.see("end")
        self.log_box.configure(state="disabled")

    def _start_install(self):
        if self.installing:
            return
        self.installing = True
        self.install_button.configure(state="disabled")
        self.close_button.configure(text="Отмена")
        self.status.configure(text="Установка выполняется…")
        self.stage_label.configure(text="Шаг 1/4 · Проверка окружения")
        self.progress.configure(mode="indeterminate")
        self.progress.start()
        self._append_log("Начало установки AUTOMATIC1111 WebUI")
        self.installer = WebUIInstaller(
            lambda line: self.events.put(("log", line)),
            lambda stage: self.events.put(("stage", stage)),
        )
        threading.Thread(target=self._install_worker, daemon=True).start()

    def _install_worker(self):
        try:
            self.installer.install()
        except InstallationCancelled as exc:
            self.events.put(("cancelled", str(exc)))
        except (InstallationError, OSError) as exc:
            self.events.put(("error", str(exc)))
        except Exception as exc:
            self.events.put(("error", f"Неожиданная ошибка: {exc}"))
        else:
            self.events.put(("done", "Установка завершена"))

    def _pump_events(self):
        if not self.winfo_exists():
            return
        lines = []
        result = None
        try:
            for _ in range(100):
                kind, message = self.events.get_nowait()
                if kind == "log":
                    lines.append(message)
                elif kind == "stage":
                    stages = {
                        "check": "Шаг 1/4 · Проверка Git и Python",
                        "clone": "Шаг 2/4 · Загрузка WebUI",
                        "venv": "Шаг 3/4 · Виртуальное окружение",
                        "dependencies": "Шаг 4/4 · Установка зависимостей",
                        "done": "Готово · пакет добавлен",
                    }
                    self.stage_label.configure(text=stages.get(message, message))
                else:
                    result = (kind, message)
        except Empty:
            pass
        if lines:
            self._append_log("\n".join(lines))
        if result is not None:
            kind, message = result
            self.installing = False
            self.progress.stop()
            self.progress.configure(mode="determinate")
            self.progress.set(1 if kind == "done" else 0)
            self._append_log(message)
            self.status.configure(text=message)
            self.close_button.configure(text="Закрыть", state="normal")
            if kind == "done":
                self.on_installed()
            elif kind in ("error", "cancelled"):
                self.install_button.configure(state="normal")
        self.after(100, self._pump_events)

    def _close(self):
        if self.installing:
            self.status.configure(text="Отмена установки…")
            self.close_button.configure(state="disabled")
            threading.Thread(target=self.installer.cancel, daemon=True).start()
            return
        self.grab_release()
        self.destroy()
