"""Вкладка поиска и скачивания моделей Civitai."""

from queue import Empty, Queue
import threading

import customtkinter as ctk

from civitai import CivitaiClient, CivitaiError, DownloadCancelled, ModelFile, models_directory


class ModelsFrame(ctk.CTkFrame):
    def __init__(self, master, app):
        super().__init__(master, fg_color="transparent")
        self.app = app
        self.client = CivitaiClient()
        self.events = Queue()
        self.cancel_event = threading.Event()
        self.busy = None
        self.file_options = {}
        self.poll_job = None
        self.indeterminate = False

        self.grid_columnconfigure(0, weight=1)

        ctk.CTkLabel(
            self, text="Загрузка моделей", text_color=app.TEXT,
            font=ctk.CTkFont(size=30, weight="bold"),
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            self, text="Найдите модель на Civitai и выберите файл для скачивания.",
            text_color=app.MUTED, font=ctk.CTkFont(size=15),
        ).grid(row=1, column=0, pady=(4, 16), sticky="w")

        search_card = self._card()
        search_card.grid(row=2, column=0, sticky="new")
        search_card.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            search_card, text="Поиск по названию или ID", text_color=app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, columnspan=2, padx=20, pady=(16, 8), sticky="w")
        self.query_entry = ctk.CTkEntry(
            search_card, height=38, placeholder_text="Название модели или числовой ID",
        )
        self.query_entry.grid(row=1, column=0, padx=(20, 8), pady=(0, 16), sticky="ew")
        self.query_entry.bind("<Return>", lambda _event: self._start_search())
        self.search_button = ctk.CTkButton(
            search_card, text="Найти", width=100, height=38, corner_radius=9,
            fg_color=app.ACCENT, hover_color=app.ACCENT_HOVER,
            command=self._start_search,
        )
        self.search_button.grid(row=1, column=1, padx=(0, 20), pady=(0, 16))

        results_card = self._card()
        results_card.grid(row=3, column=0, pady=(12, 0), sticky="nsew")
        results_card.grid_columnconfigure(0, weight=1)
        results_card.grid_rowconfigure(1, weight=1)
        ctk.CTkLabel(
            results_card, text="Результаты поиска", text_color=app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, padx=20, pady=(14, 4), sticky="w")
        self.results_list = ctk.CTkScrollableFrame(
            results_card, height=64, fg_color="transparent",
        )
        self.results_list.grid(row=1, column=0, padx=14, pady=(0, 12), sticky="nsew")
        self.results_list.grid_columnconfigure(0, weight=1)
        self._show_results_message("Введите название модели или её ID.")

        download_card = self._card()
        download_card.grid(row=4, column=0, pady=(12, 0), sticky="ew")
        download_card.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            download_card, text="Скачать файл", text_color=app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, columnspan=2, padx=20, pady=(14, 3), sticky="w")
        self.selected_label = ctk.CTkLabel(
            download_card, text="Сначала выберите модель из результатов.",
            text_color=app.MUTED, anchor="w",
        )
        self.selected_label.grid(row=1, column=0, columnspan=2, padx=20, sticky="ew")
        self.file_menu = ctk.CTkOptionMenu(
            download_card, values=["Нет выбранного файла"], height=36,
            dynamic_resizing=False, fg_color=app.HOVER,
            button_color=app.ACCENT_HOVER, button_hover_color=app.ACCENT,
            state="disabled",
        )
        self.file_menu.grid(row=2, column=0, padx=(20, 8), pady=(8, 10), sticky="ew")
        self.download_button = ctk.CTkButton(
            download_card, text="Скачать", width=108, height=36, corner_radius=9,
            fg_color=app.ACCENT, hover_color=app.ACCENT_HOVER,
            state="disabled", command=self._start_download,
        )
        self.download_button.grid(row=2, column=1, padx=(0, 20), pady=(8, 10))
        self.progress = ctk.CTkProgressBar(
            download_card, height=10, progress_color=app.ACCENT,
        )
        self.progress.grid(row=3, column=0, columnspan=2, padx=20, sticky="ew")
        self.progress.set(0)
        self.status_label = ctk.CTkLabel(
            download_card, text=f"Файлы сохраняются в {models_directory()}",
            text_color=app.MUTED, anchor="w", wraplength=700,
        )
        self.status_label.grid(row=4, column=0, padx=(20, 8), pady=(5, 14), sticky="ew")
        self.cancel_button = ctk.CTkButton(
            download_card, text="Отмена", width=108, height=30,
            fg_color="transparent", hover_color=app.HOVER,
            state="disabled", command=self._cancel_download,
        )
        self.cancel_button.grid(row=4, column=1, padx=(0, 20), pady=(5, 14))

        self.grid_rowconfigure(3, weight=1)
        self.poll_job = self.after(100, self._poll_events)

    def _card(self):
        return ctk.CTkFrame(
            self, corner_radius=18, fg_color=self.app.CARD,
            border_width=1, border_color=self.app.BORDER,
        )

    def _show_results_message(self, message):
        for widget in self.results_list.winfo_children():
            widget.destroy()
        ctk.CTkLabel(
            self.results_list, text=message, text_color=self.app.MUTED,
        ).grid(row=0, column=0, padx=8, pady=12, sticky="w")

    def _set_busy(self, operation=None):
        self.busy = operation
        self.search_button.configure(state="disabled" if operation else "normal")
        self.download_button.configure(
            state="normal" if not operation and self.file_options else "disabled"
        )
        self.cancel_button.configure(
            state="normal" if operation == "download" else "disabled"
        )

    def _start_search(self):
        query = self.query_entry.get().strip()
        if self.busy or not query:
            if not query:
                self.status_label.configure(text="Введите название модели или числовой ID.")
            return
        self.file_options = {}
        self.file_menu.configure(values=["Нет выбранного файла"], state="disabled")
        self.file_menu.set("Нет выбранного файла")
        self.selected_label.configure(text="Сначала выберите модель из результатов.")
        self._set_busy("search")
        self._show_results_message("Ищем модели на Civitai…")
        self.status_label.configure(text="Поиск моделей…")
        threading.Thread(target=self._search_worker, args=(query,), daemon=True).start()

    def _search_worker(self, query):
        try:
            models = self.client.search_models(query)
            self.events.put(("search_results", models))
        except CivitaiError as exc:
            self.events.put(("search_error", str(exc)))
        except Exception:
            self.events.put(("search_error", "Не удалось выполнить поиск моделей."))

    def _show_models(self, models):
        if not models:
            self._show_results_message("Модели не найдены.")
            self.status_label.configure(text="Поиск завершён: совпадений нет.")
            return
        for widget in self.results_list.winfo_children():
            widget.destroy()
        for row, model in enumerate(models):
            name = model.get("name", "Без названия")
            kind = model.get("type", "Модель")
            model_id = model.get("id", "?")
            ctk.CTkButton(
                self.results_list, text=f"{name}  ·  {kind}  ·  ID {model_id}",
                anchor="w", height=36, corner_radius=8,
                fg_color="transparent", hover_color=self.app.HOVER,
                command=lambda selected=model: self._select_model(selected),
            ).grid(row=row, column=0, padx=6, pady=3, sticky="ew")
        self.status_label.configure(text=f"Найдено моделей: {len(models)}. Выберите модель.")

    def _select_model(self, model):
        self.selected_label.configure(
            text=f"{model.get('name', 'Модель')}  ·  {model.get('type', '')}"
        )
        self.file_options = {}
        versions = sorted(
            model.get("modelVersions") or [],
            key=lambda version: version.get("publishedAt") or version.get("createdAt") or "",
            reverse=True,
        )
        for version in versions:
            files = sorted(version.get("files") or [], key=lambda item: not item.get("primary"))
            for item in files:
                name = item.get("name")
                url = item.get("downloadUrl")
                if not name or not url:
                    continue
                label = f"{version.get('name', 'Версия')} · {name}"
                if label in self.file_options:
                    label = f"{label} · {item.get('id', len(self.file_options) + 1)}"
                try:
                    size_kb = float(item.get("sizeKB") or 0)
                except (TypeError, ValueError):
                    size_kb = 0
                self.file_options[label] = ModelFile(name, url, model.get("type", ""), size_kb)

        if self.file_options:
            labels = list(self.file_options)
            self.file_menu.configure(values=labels, state="normal")
            self.file_menu.set(labels[0])
            self.status_label.configure(text="Выберите файл и нажмите «Скачать».")
        else:
            self.file_menu.configure(values=["Нет доступных файлов"], state="disabled")
            self.file_menu.set("Нет доступных файлов")
            self.status_label.configure(text="У этой модели нет доступных файлов.")
        self._set_busy(self.busy)

    def _start_download(self):
        if self.busy:
            return
        model_file = self.file_options.get(self.file_menu.get())
        if model_file is None:
            return
        self.cancel_event.clear()
        self._set_busy("download")
        self.progress.stop()
        self.progress.configure(mode="determinate")
        self.progress.set(0)
        self.indeterminate = False
        self.status_label.configure(text=f"Скачиваем {model_file.name}…")
        threading.Thread(
            target=self._download_worker, args=(model_file,), daemon=True
        ).start()

    def _download_worker(self, model_file):
        try:
            target = self.client.download_file(
                model_file,
                lambda count, total, speed: self.events.put(
                    ("progress", count, total, speed)
                ),
                self.cancel_event,
            )
            self.events.put(("download_done", str(target)))
        except DownloadCancelled:
            self.events.put(("download_cancelled",))
        except (CivitaiError, OSError) as exc:
            self.events.put(("download_error", str(exc)))
        except Exception:
            self.events.put(("download_error", "Не удалось скачать файл модели."))

    def _cancel_download(self):
        self.cancel_event.set()
        self.status_label.configure(text="Останавливаем загрузку…")
        self.cancel_button.configure(state="disabled")

    def _poll_events(self):
        self.poll_job = None
        try:
            while True:
                event = self.events.get_nowait()
                kind = event[0]
                if kind == "search_results":
                    self._set_busy()
                    self._show_models(event[1])
                elif kind == "search_error":
                    self._set_busy()
                    self._show_results_message(event[1])
                    self.status_label.configure(text=event[1])
                elif kind == "progress":
                    self._show_progress(*event[1:])
                elif kind in ("download_done", "download_error", "download_cancelled"):
                    self._finish_download(kind, event[1] if len(event) > 1 else None)
        except Empty:
            pass
        self.poll_job = self.after(100, self._poll_events)

    def _show_progress(self, downloaded, total, speed):
        amount = downloaded / (1024 * 1024)
        rate = speed / (1024 * 1024)
        if total:
            if self.indeterminate:
                self.progress.stop()
                self.progress.configure(mode="determinate")
                self.indeterminate = False
            fraction = min(downloaded / total, 0.99)
            self.progress.set(fraction)
            self.status_label.configure(
                text=f"{fraction:.0%} · {amount:.1f}/{total / (1024 * 1024):.1f} МБ · {rate:.1f} МБ/с"
            )
        else:
            if not self.indeterminate:
                self.progress.configure(mode="indeterminate")
                self.progress.start()
                self.indeterminate = True
            self.status_label.configure(text=f"{amount:.1f} МБ · {rate:.1f} МБ/с")

    def _finish_download(self, kind, message):
        if self.indeterminate:
            self.progress.stop()
            self.progress.configure(mode="determinate")
            self.indeterminate = False
        if kind == "download_done":
            self.progress.set(1)
            self.status_label.configure(text=f"Готово: {message}")
        else:
            self.progress.set(0)
            self.status_label.configure(
                text=message if kind == "download_error" else "Загрузка отменена."
            )
        self._set_busy()

    def stop(self):
        self.cancel_event.set()
        if self.poll_job is not None:
            self.after_cancel(self.poll_job)
            self.poll_job = None
