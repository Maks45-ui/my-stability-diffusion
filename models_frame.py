"""Вкладка поиска и скачивания моделей Civitai."""

from queue import Empty, Queue
import threading

import customtkinter as ctk
from PIL import Image, ImageOps

from civitai import (
    CivitaiClient, CivitaiError, DownloadCancelled, ModelFile,
    model_images, models_directory,
)


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
        self.search_job = None
        self.search_generation = 0
        self.image_tasks = Queue()
        self.image_targets = {}
        self.thumbnail_refs = {}
        self.model_cards = []
        self.card_by_model = {}
        self.catalog_columns = 1
        self.viewer = None
        self.viewer_generation = 0
        self.stopping = threading.Event()
        self.indeterminate = False
        for _ in range(4):
            threading.Thread(target=self._image_worker, daemon=True).start()

        self.grid_columnconfigure(0, weight=3)
        self.grid_columnconfigure(1, weight=2)

        ctk.CTkLabel(
            self, text="Загрузка моделей", text_color=app.TEXT,
            font=ctk.CTkFont(size=30, weight="bold"),
        ).grid(row=0, column=0, columnspan=2, sticky="w")
        ctk.CTkLabel(
            self, text="Найдите модель на Civitai и выберите файл для скачивания.",
            text_color=app.MUTED, font=ctk.CTkFont(size=15),
        ).grid(row=1, column=0, columnspan=2, pady=(4, 16), sticky="w")

        search_card = self._card()
        search_card.grid(row=2, column=0, columnspan=2, sticky="ew")
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
        self.query_entry.bind("<KeyRelease>", self._schedule_search)
        self.search_button = ctk.CTkButton(
            search_card, text="Найти", width=100, height=38, corner_radius=9,
            fg_color=app.ACCENT, hover_color=app.ACCENT_HOVER,
            command=self._start_search,
        )
        self.search_button.grid(row=1, column=1, padx=(0, 20), pady=(0, 16))

        results_card = self._card()
        results_card.grid(row=3, column=0, padx=(0, 12), pady=(12, 0), sticky="nsew")
        results_card.grid_columnconfigure(0, weight=1)
        results_card.grid_rowconfigure(1, weight=1)
        ctk.CTkLabel(
            results_card, text="Каталог Civitai", text_color=app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, padx=20, pady=(14, 4), sticky="w")
        self.results_list = ctk.CTkScrollableFrame(
            results_card, height=64, fg_color="transparent",
        )
        self.results_list.grid(row=1, column=0, padx=14, pady=(0, 12), sticky="nsew")
        self.results_list.grid_columnconfigure(0, weight=1)
        self.results_list.grid_columnconfigure(1, weight=1)
        self.results_list.bind("<Configure>", self._layout_model_cards)
        self._show_loading_skeletons()

        details = ctk.CTkScrollableFrame(self, fg_color="transparent")
        details.grid(row=3, column=1, pady=(12, 0), sticky="nsew")
        details.grid_columnconfigure(0, weight=1)
        self.preview_card = self._card(details)
        self.preview_card.grid(row=0, column=0, sticky="ew")
        self.preview_card.grid_columnconfigure(0, weight=1)
        self._show_preview_message()

        download_card = self._card(details)
        download_card.grid(row=1, column=0, pady=(12, 0), sticky="ew")
        download_card.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            download_card, text="Скачать файл", text_color=app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, columnspan=2, padx=20, pady=(14, 3), sticky="w")
        self.selected_label = ctk.CTkLabel(
            download_card, text="Сначала выберите модель из результатов.",
            text_color=app.MUTED, anchor="w", wraplength=210, justify="left",
        )
        self.selected_label.grid(row=1, column=0, columnspan=2, padx=20, sticky="ew")
        self.file_menu = ctk.CTkOptionMenu(
            download_card, values=["Нет выбранного файла"], height=36,
            dynamic_resizing=False, fg_color=app.HOVER,
            button_color=app.ACCENT_HOVER, button_hover_color=app.ACCENT,
            state="disabled",
        )
        self.file_menu.grid(row=2, column=0, columnspan=2,
                            padx=20, pady=(8, 10), sticky="ew")
        self.download_button = ctk.CTkButton(
            download_card, text="Скачать", width=108, height=36, corner_radius=9,
            fg_color=app.ACCENT, hover_color=app.ACCENT_HOVER,
            state="disabled", command=self._start_download,
        )
        self.download_button.grid(row=3, column=0, columnspan=2,
                                  padx=20, pady=(0, 12), sticky="ew")
        self.progress = ctk.CTkProgressBar(
            download_card, height=10, progress_color=app.ACCENT,
        )
        self.progress.grid(row=4, column=0, columnspan=2, padx=20, sticky="ew")
        self.progress.set(0)
        self.status_label = ctk.CTkLabel(
            download_card, text=f"Файлы сохраняются в {models_directory()}",
            text_color=app.MUTED, anchor="w", wraplength=210, justify="left",
        )
        self.status_label.grid(row=5, column=0, columnspan=2,
                               padx=20, pady=(8, 2), sticky="ew")
        self.cancel_button = ctk.CTkButton(
            download_card, text="Отмена", width=108, height=30,
            fg_color="transparent", hover_color=app.HOVER,
            state="disabled", command=self._cancel_download,
        )
        self.cancel_button.grid(row=6, column=0, columnspan=2,
                                padx=20, pady=(0, 16), sticky="ew")

        self.grid_rowconfigure(3, weight=1)
        self.poll_job = self.after(100, self._poll_events)
        self._start_search()

    def _card(self, parent=None):
        return ctk.CTkFrame(
            parent or self, corner_radius=16, fg_color=self.app.CARD,
        )

    def _show_results_message(self, message):
        self.model_cards = []
        self.card_by_model = {}
        for widget in self.results_list.winfo_children():
            widget.destroy()
        ctk.CTkLabel(
            self.results_list, text=message, text_color=self.app.MUTED,
        ).grid(row=0, column=0, padx=8, pady=12, sticky="w")

    def _show_loading_skeletons(self):
        self.model_cards = []
        self.card_by_model = {}
        for widget in self.results_list.winfo_children():
            widget.destroy()
        for index in range(4):
            tile = ctk.CTkFrame(self.results_list, fg_color=self.app.SURFACE, corner_radius=12)
            tile.grid(row=index // 2, column=index % 2, padx=6, pady=6, sticky="ew")
            tile.grid_columnconfigure(0, weight=1)
            ctk.CTkFrame(
                tile, height=130, fg_color=self.app.HOVER, corner_radius=9,
            ).grid(row=0, column=0, padx=10, pady=(10, 12), sticky="ew")
            ctk.CTkFrame(
                tile, height=12, width=112, fg_color=self.app.HOVER, corner_radius=5,
            ).grid(row=1, column=0, padx=10, pady=(0, 8), sticky="w")
            ctk.CTkFrame(
                tile, height=9, width=72, fg_color=self.app.HOVER, corner_radius=5,
            ).grid(row=2, column=0, padx=10, pady=(0, 12), sticky="w")

    def _show_preview_message(self):
        for widget in self.preview_card.winfo_children():
            widget.destroy()
        ctk.CTkLabel(
            self.preview_card, text="Детали модели", text_color=self.app.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, padx=18, pady=(18, 8), sticky="w")
        ctk.CTkLabel(
            self.preview_card,
            text="Выберите обложку в каталоге, чтобы увидеть все примеры и доступные файлы.",
            text_color=self.app.MUTED, wraplength=210, justify="left",
        ).grid(row=1, column=0, padx=18, pady=(0, 18), sticky="w")

    def _layout_model_cards(self, _event=None):
        columns = 2 if self.results_list.winfo_width() >= 445 else 1
        if columns == self.catalog_columns:
            return
        self.catalog_columns = columns
        self.results_list.grid_columnconfigure(1, weight=1 if columns == 2 else 0)
        for index, card in enumerate(self.model_cards):
            if card.winfo_exists():
                card.grid(row=index // columns, column=index % columns,
                          padx=6, pady=6, sticky="ew")

    def _set_busy(self, operation=None):
        self.busy = operation
        self.query_entry.configure(
            state="disabled" if operation == "download" else "normal"
        )
        self.search_button.configure(state="disabled" if operation else "normal")
        self.download_button.configure(
            state="normal" if not operation and self.file_options else "disabled"
        )
        self.cancel_button.configure(
            state="normal" if operation == "download" else "disabled"
        )

    def _schedule_search(self, event):
        if event.keysym in ("Return", "Up", "Down", "Left", "Right"):
            return
        if self.search_job is not None:
            self.after_cancel(self.search_job)
        self.search_job = self.after(500, self._start_search)

    def _start_search(self):
        if self.search_job is not None:
            self.after_cancel(self.search_job)
            self.search_job = None
        query = self.query_entry.get().strip()
        if self.busy == "download":
            return
        self.search_generation += 1
        generation = self.search_generation
        self.image_targets.clear()
        self.thumbnail_refs.clear()
        self.file_options = {}
        self.file_menu.configure(values=["Нет выбранного файла"], state="disabled")
        self.file_menu.set("Нет выбранного файла")
        self.selected_label.configure(text="Сначала выберите модель из результатов.")
        self._set_busy("search")
        self._show_loading_skeletons()
        self._show_preview_message()
        self.status_label.configure(text="Поиск моделей…" if query else "Загрузка ленты…")
        threading.Thread(
            target=self._search_worker, args=(query, generation), daemon=True
        ).start()

    def _search_worker(self, query, generation):
        try:
            models = self.client.search_models(query)
            self.events.put(("search_results", generation, models))
        except CivitaiError as exc:
            self.events.put(("search_error", generation, str(exc)))
        except Exception:
            self.events.put(("search_error", generation, "Не удалось выполнить поиск моделей."))

    def _show_models(self, models):
        if not models:
            self._show_results_message("Модели не найдены.")
            self.status_label.configure(text="Поиск завершён: совпадений нет.")
            return
        for widget in self.results_list.winfo_children():
            widget.destroy()
        self.model_cards = []
        self.card_by_model = {}
        self.catalog_columns = 2 if self.results_list.winfo_width() >= 445 else 1
        self.results_list.grid_columnconfigure(1, weight=1 if self.catalog_columns == 2 else 0)
        for index, model in enumerate(models):
            name = str(model.get("name") or "Без названия")
            kind = model.get("type", "Модель")
            model_id = model.get("id", "?")
            images = model_images(model)
            state = {"model": model, "images": images}
            card = ctk.CTkFrame(
                self.results_list, corner_radius=12, fg_color=self.app.SURFACE,
            )
            card.grid(row=index // self.catalog_columns, column=index % self.catalog_columns,
                      padx=6, pady=6, sticky="ew")
            card.grid_columnconfigure(0, weight=1)
            self.model_cards.append(card)
            self.card_by_model[id(model)] = card
            cover = ctk.CTkFrame(card, height=164, fg_color=self.app.HOVER, corner_radius=10)
            cover.grid(row=0, column=0, padx=9, pady=(9, 0), sticky="ew")
            cover.grid_propagate(False)
            cover_button = ctk.CTkButton(
                cover, text="▧" if not images else "Загрузка…",
                corner_radius=9, fg_color=self.app.HOVER,
                hover_color=self.app.ACCENT_HOVER, text_color=self.app.MUTED,
                command=(lambda current=state: self._open_cover(current))
                if images else (lambda selected=model: self._select_model(selected)),
            )
            cover_button.place(relx=0, rely=0, relwidth=1, relheight=1)
            ctk.CTkButton(
                cover, text=f"{name[:28]}\n{kind}",
                anchor="w", height=56, corner_radius=0,
                fg_color="#17212C", hover_color="#203246",
                text_color=self.app.TEXT, font=ctk.CTkFont(size=13, weight="bold"),
                command=lambda selected=model: self._select_model(selected),
            ).place(relx=0, rely=1, anchor="sw", relwidth=1)
            if images:
                image_id = id(cover_button)
                self.image_targets[image_id] = cover_button
                self.image_tasks.put((self.search_generation, image_id, images[0],
                                      (240, 164), "cover"))
            ctk.CTkLabel(
                card, text=f"ID {model_id}  ·  {len(images)} примеров",
                text_color=self.app.MUTED, font=ctk.CTkFont(size=11),
            ).grid(row=1, column=0, padx=12, pady=(8, 4), sticky="w")
            ctk.CTkButton(
                card, text="Скачать модель  ↓", height=34, corner_radius=8,
                fg_color=self.app.ACCENT, hover_color=self.app.ACCENT_HOVER,
                command=lambda selected=model: self._download_model(selected),
            ).grid(row=2, column=0, padx=9, pady=(0, 9), sticky="ew")
        self.status_label.configure(text=f"Моделей в каталоге: {len(models)}. Выберите обложку или модель.")

    def _show_selected_preview(self, model):
        for widget in self.preview_card.winfo_children():
            widget.destroy()
        images = model_images(model)
        ctk.CTkLabel(
            self.preview_card, text=str(model.get("name") or "Модель")[:55],
            text_color=self.app.TEXT, font=ctk.CTkFont(size=17, weight="bold"),
            wraplength=210, justify="left",
        ).grid(row=0, column=0, padx=18, pady=(18, 3), sticky="w")
        ctk.CTkLabel(
            self.preview_card,
            text=f"{model.get('type', 'Модель')}  ·  ID {model.get('id', '?')}  ·  {len(images)} примеров",
            text_color=self.app.MUTED, font=ctk.CTkFont(size=11),
        ).grid(row=1, column=0, padx=18, pady=(0, 12), sticky="w")
        if not images:
            ctk.CTkLabel(
                self.preview_card, text="Примеров изображений нет", text_color=self.app.MUTED,
            ).grid(row=2, column=0, padx=18, pady=(0, 18), sticky="w")
            return
        gallery = ctk.CTkScrollableFrame(
            self.preview_card, orientation="horizontal", height=110,
            fg_color="transparent",
        )
        gallery.grid(row=2, column=0, padx=12, sticky="ew")
        more_button = ctk.CTkButton(
            self.preview_card, text="", height=28, anchor="w", fg_color="transparent",
            hover_color=self.app.HOVER, text_color=self.app.ACCENT,
        )
        more_button.grid(row=3, column=0, padx=18, pady=(4, 12), sticky="w")
        state = {"gallery": gallery, "more": more_button,
                 "model": model, "images": images, "shown": 0}
        more_button.configure(command=lambda: self._add_image_batch(state))
        self._add_image_batch(state)

    def _add_image_batch(self, state):
        images = state["images"]
        start = state["shown"]
        for index in range(start, min(start + 6, len(images))):
            button = ctk.CTkButton(
                state["gallery"], text="Загрузка…", width=150, height=104,
                corner_radius=8, fg_color=self.app.CARD,
                hover_color=self.app.ACCENT_HOVER, text_color=self.app.MUTED,
                command=lambda current=state, number=index: self._open_viewer(current, number),
            )
            button.grid(row=0, column=index, padx=4, pady=2)
            image_id = id(button)
            self.image_targets[image_id] = button
            self.image_tasks.put((self.search_generation, image_id, images[index],
                                  (150, 100), "preview"))
        state["shown"] = min(start + 6, len(images))
        remaining = len(images) - state["shown"]
        if remaining:
            state["more"].configure(text=f"Ещё изображения ({remaining})")
        else:
            state["more"].grid_remove()

    def _image_worker(self):
        while not self.stopping.is_set():
            try:
                generation, image_id, url, size, style = self.image_tasks.get(timeout=0.2)
            except Empty:
                continue
            if generation != self.search_generation:
                continue
            try:
                picture = self.client.load_image(url, size)
                if style == "cover":
                    picture = ImageOps.fit(picture, size, method=Image.Resampling.LANCZOS)
                self.events.put(("thumbnail", generation, image_id, picture))
            except Exception:
                self.events.put(("thumbnail_error", generation, image_id))

    def _open_cover(self, state):
        self._select_model(state["model"])
        self._open_viewer(state, 0)

    def _open_viewer(self, state, index):
        self._close_viewer()
        window = ctk.CTkToplevel(self, fg_color=self.app.BACKGROUND)
        window.title(f"Flow · {state['model'].get('name', 'Пример')}")
        window.geometry("980x740")
        window.minsize(700, 520)
        window.grid_columnconfigure(0, weight=1)
        window.grid_rowconfigure(1, weight=1)
        window.protocol("WM_DELETE_WINDOW", self._close_viewer)

        ctk.CTkLabel(
            window, text=str(state["model"].get("name") or "Модель")[:100],
            text_color=self.app.TEXT, font=ctk.CTkFont(size=20, weight="bold"),
        ).grid(row=0, column=0, padx=24, pady=(18, 6), sticky="w")
        image_label = ctk.CTkLabel(
            window, text="Загружаем изображение…", text_color=self.app.MUTED,
        )
        image_label.grid(row=1, column=0, padx=20, pady=8)

        controls = ctk.CTkFrame(window, fg_color="transparent")
        controls.grid(row=2, column=0, padx=20, pady=(6, 20))
        ctk.CTkButton(
            controls, text="←", width=52, command=lambda: self._move_viewer(-1),
        ).grid(row=0, column=0, padx=8)
        counter = ctk.CTkLabel(controls, text="", width=95, text_color=self.app.TEXT)
        counter.grid(row=0, column=1, padx=8)
        ctk.CTkButton(
            controls, text="→", width=52, command=lambda: self._move_viewer(1),
        ).grid(row=0, column=2, padx=8)
        ctk.CTkButton(
            controls, text="Скачать модель", width=150,
            fg_color=self.app.ACCENT, hover_color=self.app.ACCENT_HOVER,
            command=lambda model=state["model"]: self._download_from_viewer(model),
        ).grid(row=0, column=3, padx=(24, 8))

        self.viewer = {
            "window": window, "label": image_label, "counter": counter,
            "model": state["model"], "images": state["images"],
            "index": index, "photo": None,
        }
        self._request_viewer_image()
        window.lift()
        window.focus()

    def _move_viewer(self, offset):
        if self.viewer is None:
            return
        count = len(self.viewer["images"])
        self.viewer["index"] = (self.viewer["index"] + offset) % count
        self._request_viewer_image()

    def _request_viewer_image(self):
        self.viewer_generation += 1
        generation = self.viewer_generation
        index = self.viewer["index"]
        self.viewer["counter"].configure(text=f"{index + 1} / {len(self.viewer['images'])}")
        self.viewer["label"].configure(image=None, text="Загружаем изображение…")
        self.viewer["photo"] = None
        url = self.viewer["images"][index]
        threading.Thread(
            target=self._viewer_worker, args=(generation, url), daemon=True
        ).start()

    def _viewer_worker(self, generation, url):
        try:
            picture = self.client.load_image(url, (900, 590))
            self.events.put(("viewer_image", generation, picture))
        except Exception:
            self.events.put(("viewer_error", generation))

    def _close_viewer(self):
        self.viewer_generation += 1
        if self.viewer is not None:
            self.viewer["window"].destroy()
            self.viewer = None

    def _download_model(self, model):
        if self.busy:
            return
        self._select_model(model)
        if self.file_options:
            self._start_download()

    def _download_from_viewer(self, model):
        if self.busy:
            return
        self._download_model(model)
        if self.busy == "download":
            self._close_viewer()

    def _select_model(self, model):
        for card in self.model_cards:
            if card.winfo_exists():
                selected = card is self.card_by_model.get(id(model))
                card.configure(border_width=2 if selected else 0,
                               border_color=self.app.ACCENT if selected else self.app.SURFACE)
        self._show_selected_preview(model)
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
        for _ in range(24):
            try:
                event = self.events.get_nowait()
            except Empty:
                break
            kind = event[0]
            if kind == "search_results":
                if event[1] == self.search_generation:
                    self._set_busy()
                    self._show_models(event[2])
            elif kind == "search_error":
                if event[1] == self.search_generation:
                    self._set_busy()
                    self._show_results_message(event[2])
                    self.status_label.configure(text=event[2])
            elif kind == "thumbnail":
                if event[1] == self.search_generation:
                    button = self.image_targets.get(event[2])
                    if button is not None and button.winfo_exists():
                        picture = event[3]
                        photo = ctk.CTkImage(
                            light_image=picture, dark_image=picture, size=picture.size,
                        )
                        self.thumbnail_refs[event[2]] = photo
                        button.configure(image=photo, text="")
            elif kind == "thumbnail_error":
                if event[1] == self.search_generation:
                    button = self.image_targets.get(event[2])
                    if button is not None and button.winfo_exists():
                        button.configure(text="Нет превью")
            elif kind == "viewer_image":
                if self.viewer is not None and event[1] == self.viewer_generation:
                    picture = event[2]
                    photo = ctk.CTkImage(
                        light_image=picture, dark_image=picture, size=picture.size,
                    )
                    self.viewer["photo"] = photo
                    self.viewer["label"].configure(image=photo, text="")
            elif kind == "viewer_error":
                if self.viewer is not None and event[1] == self.viewer_generation:
                    self.viewer["label"].configure(text="Не удалось загрузить изображение.")
            elif kind == "progress":
                self._show_progress(*event[1:])
            elif kind in ("download_done", "download_error", "download_cancelled"):
                self._finish_download(kind, event[1] if len(event) > 1 else None)
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
        self.stopping.set()
        self.cancel_event.set()
        if self.search_job is not None:
            self.after_cancel(self.search_job)
            self.search_job = None
        if self.poll_job is not None:
            self.after_cancel(self.poll_job)
            self.poll_job = None
        self._close_viewer()
