from pathlib import Path
import sys
import threading
from tkinter import TclError

import customtkinter as ctk
from PIL import Image, ImageTk

from discord_presence import DiscordPresence
from install_dialog import InstallDialog
from installer import (
    InstallationError, installed_packages, launch_webui, open_package_settings,
    package_version,
)
from models_frame import ModelsFrame


ctk.set_appearance_mode("dark")
ctk.set_default_color_theme("blue")


class SettingsFrame(ctk.CTkFrame):
    def __init__(self, master, app):
        super().__init__(master, fg_color="transparent")
        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(2, weight=1)

        ctk.CTkLabel(
            self,
            text="Настройки",
            text_color=app.TEXT,
            font=ctk.CTkFont(size=30, weight="bold"),
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            self,
            text="Основные параметры Flow.",
            text_color=app.MUTED,
            font=ctk.CTkFont(size=15),
        ).grid(row=1, column=0, pady=(4, 22), sticky="w")

        card = ctk.CTkFrame(
            self,
            corner_radius=16,
            fg_color=app.CARD,
        )
        card.grid(row=2, column=0, sticky="nsew")
        card.grid_columnconfigure(0, weight=1)
        card.grid_rowconfigure(4, weight=1)

        ctk.CTkLabel(
            card,
            text="Основные параметры",
            text_color=app.TEXT,
            font=ctk.CTkFont(size=20, weight="bold"),
        ).grid(row=0, column=0, padx=28, pady=(24, 22), sticky="w")

        settings = (
            ("Тема интерфейса", "Тёмная"),
            ("Язык", "Русский"),
            ("Установка пакетов", "AUTOMATIC1111 WebUI"),
        )
        for row, (title, value) in enumerate(settings, start=1):
            option = ctk.CTkFrame(card, corner_radius=10, fg_color=app.SURFACE)
            option.grid(row=row, column=0, padx=28, pady=(0, 8), sticky="ew")
            option.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(
                option, text=title, text_color=app.TEXT, font=ctk.CTkFont(size=14)
            ).grid(row=0, column=0, padx=18, pady=14, sticky="w")
            ctk.CTkLabel(
                option, text=value, text_color=app.MUTED, font=ctk.CTkFont(size=14)
            ).grid(row=0, column=1, padx=18, pady=14, sticky="e")

        ctk.CTkLabel(
            card,
            text="Изменение параметров появится позже.",
            text_color=app.MUTED,
            font=ctk.CTkFont(size=13),
        ).grid(row=5, column=0, padx=28, pady=(20, 28), sticky="w")


class FlowApp(ctk.CTk):
    BACKGROUND = "#191D23"
    SIDEBAR = "#20252D"
    CARD = "#242B34"
    SURFACE = "#2C3540"
    BORDER = "#3B4653"
    HOVER = "#34404D"
    ACCENT = "#2B82C5"
    ACCENT_HOVER = "#236FA9"
    TEXT = "#F4F7FB"
    MUTED = "#A9B5C2"
    SUCCESS = "#58D2A5"

    EXPANDED_WIDTH = 224
    COLLAPSED_WIDTH = 72
    NAV_HIGHLIGHT_STEPS = 5
    NAV_HIGHLIGHT_FRAME_MS = 20
    NAV_ITEMS = (
        ("Главная", "home.png", "⌂"),
        ("Галерея", "gallery.png", "▦"),
        ("Загрузка моделей", "download.png", "↓"),
        ("Настройки", "settings.png", "⚙"),
    )
    SUBTITLES = {
        "Главная": "Управляйте пакетами и инструментами в одном месте.",
        "Галерея": "Здесь будут собраны ваши сгенерированные изображения.",
        "Загрузка моделей": "Каталог и загрузки моделей с Civitai.",
    }

    def __init__(self):
        super().__init__()
        self.title("Flow")
        self.geometry("1180x720")
        self.minsize(900, 600)
        self.configure(fg_color=self.BACKGROUND)
        self.grid_rowconfigure(0, weight=1)
        self.grid_columnconfigure(0, weight=0)
        self.grid_columnconfigure(1, weight=1)

        self.asset_root = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))
        self.logo_image = None
        self.window_icon = None
        self.nav_icons = {}
        self._load_logo()
        self._load_nav_icons()

        self.sidebar_expanded = True
        self.active_page = "Главная"
        self.nav_buttons = {}
        self._nav_colors = {}
        self._nav_highlight_job = None
        self.models_frame = None
        self.install_dialog = None
        self.package_settings_dialog = None
        self.webui_process = None
        self.package_error = None
        self._package_poll_job = None
        self.package_badge = None
        self.package_start_button = None
        self.discord_presence = DiscordPresence()

        self._build_sidebar()
        self._build_main_area()
        self.show_page("Главная")
        self._sync_package_status()
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _load_logo(self):
        for logo_path in (
            self.asset_root / "assets" / "logo.png",
            self.asset_root / "logo.png",
        ):
            try:
                with Image.open(logo_path) as source:
                    logo = source.convert("RGBA")
                self.logo_image = ctk.CTkImage(
                    light_image=logo, dark_image=logo, size=(44, 44)
                )
                self.window_icon = ImageTk.PhotoImage(logo)
                self.after(250, self._set_window_icon)
                return
            except (OSError, ValueError, TclError):
                continue

    def _load_nav_icons(self):
        for name, filename, _ in self.NAV_ITEMS:
            try:
                with Image.open(self.asset_root / "assets" / "icons" / filename) as source:
                    icon = source.convert("RGBA")
                bounds = icon.getchannel("A").getbbox()
                if bounds:
                    icon = icon.crop(bounds)
                    scale = 68 / max(icon.size)
                    icon = icon.resize(
                        (round(icon.width * scale), round(icon.height * scale)),
                        Image.Resampling.LANCZOS,
                    )
                    canvas = Image.new("RGBA", (96, 96), (0, 0, 0, 0))
                    canvas.alpha_composite(
                        icon, ((96 - icon.width) // 2, (96 - icon.height) // 2),
                    )
                    icon = canvas
                self.nav_icons[name] = ctk.CTkImage(
                    light_image=icon, dark_image=icon, size=(20, 20)
                )
            except (OSError, ValueError):
                self.nav_icons[name] = None

    def _set_window_icon(self):
        if self.window_icon is not None:
            try:
                self.wm_iconphoto(True, self.window_icon)
            except TclError:
                pass

    def _build_sidebar(self):
        self.sidebar = ctk.CTkFrame(
            self, width=self.EXPANDED_WIDTH, corner_radius=0, fg_color=self.SIDEBAR
        )
        self.sidebar.grid(row=0, column=0, sticky="nsew")
        self.sidebar.grid_propagate(False)
        self.sidebar.grid_columnconfigure(0, weight=1)
        self.sidebar.grid_rowconfigure(4, weight=1)

        ctk.CTkButton(
            self.sidebar,
            text="☰",
            width=46,
            height=42,
            corner_radius=10,
            fg_color="transparent",
            hover_color=self.HOVER,
            text_color=self.TEXT,
            font=ctk.CTkFont(size=22),
            command=self.toggle_sidebar,
        ).grid(row=0, column=0, padx=13, pady=(22, 32), sticky="w")

        for row, (name, _, fallback) in enumerate(self.NAV_ITEMS[:3], start=1):
            self._add_nav_button(name, fallback, row)
        name, _, fallback = self.NAV_ITEMS[3]
        self._add_nav_button(name, fallback, 5)

    def _add_nav_button(self, name, fallback, row):
        icon = self.nav_icons[name]
        button = ctk.CTkButton(
            self.sidebar,
            width=46,
            height=46,
            corner_radius=10,
            border_spacing=9,
            text=name if icon is not None else f"{fallback}  {name}",
            image=icon,
            compound="left",
            anchor="w",
            fg_color="transparent",
            hover_color=self.HOVER,
            text_color=self.TEXT,
            font=ctk.CTkFont(size=15),
            command=lambda page=name: self.show_page(page),
        )
        button.grid(row=row, column=0, padx=12, pady=4, sticky="ew")
        self.nav_buttons[name] = (button, fallback)

    def _build_main_area(self):
        main = ctk.CTkFrame(self, corner_radius=0, fg_color=self.BACKGROUND)
        main.grid(row=0, column=1, sticky="nsew")
        main.grid_columnconfigure(0, weight=1)
        main.grid_rowconfigure(2, weight=1)

        header = ctk.CTkFrame(main, height=82, corner_radius=0, fg_color=self.BACKGROUND)
        header.grid(row=0, column=0, sticky="ew")
        header.grid_propagate(False)
        header.grid_columnconfigure(1, weight=1)

        if self.logo_image is not None:
            logo_label = ctk.CTkLabel(header, image=self.logo_image, text="")
        else:
            logo_label = ctk.CTkLabel(
                header,
                text="F",
                width=44,
                height=44,
                corner_radius=13,
                fg_color=self.ACCENT,
                text_color=self.TEXT,
                font=ctk.CTkFont(size=25, weight="bold"),
            )
        logo_label.grid(row=0, column=0, rowspan=2, padx=(28, 12), pady=19)

        ctk.CTkLabel(
            header,
            text="Flow",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=23, weight="bold"),
        ).grid(row=0, column=1, sticky="sw", pady=(14, 0))
        ctk.CTkLabel(
            header,
            text="Творчество под вашим контролем",
            text_color=self.MUTED,
            font=ctk.CTkFont(size=12),
        ).grid(row=1, column=1, sticky="nw", pady=(0, 14))

        ctk.CTkFrame(main, height=1, corner_radius=0, fg_color=self.BORDER).grid(
            row=1, column=0, sticky="ew"
        )

        self.page_container = ctk.CTkFrame(main, fg_color="transparent")
        self.page_container.grid(row=2, column=0, padx=28, pady=(24, 24), sticky="nsew")
        self.page_container.grid_columnconfigure(0, weight=1)
        self.page_container.grid_rowconfigure(2, weight=1)

    def toggle_sidebar(self):
        self.sidebar_expanded = not self.sidebar_expanded
        if self.sidebar_expanded:
            self.sidebar.configure(width=self.EXPANDED_WIDTH)
        else:
            for button, fallback in self.nav_buttons.values():
                button.configure(
                    text="" if button.cget("image") is not None else fallback,
                    anchor="center",
                )
            self.sidebar.configure(width=self.COLLAPSED_WIDTH)
            return

        for name, (button, fallback) in self.nav_buttons.items():
            button.configure(
                text=name if button.cget("image") is not None else f"{fallback}  {name}",
                anchor="w",
            )

    @staticmethod
    def _blend_color(start, end, amount):
        channels = (
            round(int(start[index:index + 2], 16) * (1 - amount)
                  + int(end[index:index + 2], 16) * amount)
            for index in (1, 3, 5)
        )
        return "#" + "".join(f"{channel:02X}" for channel in channels)

    def _refresh_navigation(self, animate=False):
        if not animate and self._nav_highlight_job is not None:
            return

        if self._nav_highlight_job is not None:
            self.after_cancel(self._nav_highlight_job)
            self._nav_highlight_job = None

        targets = {
            name: self.ACCENT_HOVER if name == self.active_page else self.SIDEBAR
            for name in self.nav_buttons
        }
        if not animate or not self._nav_colors:
            self._nav_colors = targets
            for name, (button, _) in self.nav_buttons.items():
                selected = name == self.active_page
                button.configure(
                    fg_color=self.ACCENT_HOVER if selected else "transparent",
                    hover_color=self.ACCENT if selected else self.HOVER,
                    text_color=self.TEXT,
                )
            return

        self._nav_start_colors = self._nav_colors.copy()
        self._nav_target_colors = targets
        self._nav_highlight_step = 0
        self._nav_highlight_job = self.after(
            self.NAV_HIGHLIGHT_FRAME_MS, self._animate_nav_highlight
        )

    def _animate_nav_highlight(self):
        self._nav_highlight_job = None
        self._nav_highlight_step += 1
        progress = self._nav_highlight_step / self.NAV_HIGHLIGHT_STEPS
        eased = progress * progress * (3 - 2 * progress)

        for name, (button, _) in self.nav_buttons.items():
            color = self._blend_color(
                self._nav_start_colors[name], self._nav_target_colors[name], eased
            )
            self._nav_colors[name] = color
            button.configure(fg_color=color, hover_color=color)

        if self._nav_highlight_step < self.NAV_HIGHLIGHT_STEPS:
            self._nav_highlight_job = self.after(
                self.NAV_HIGHLIGHT_FRAME_MS, self._animate_nav_highlight
            )
        else:
            for name, (button, _) in self.nav_buttons.items():
                selected = name == self.active_page
                button.configure(
                    fg_color=self.ACCENT_HOVER if selected else "transparent",
                    hover_color=self.ACCENT if selected else self.HOVER,
                )

    def show_page(self, page_name):
        animate = page_name != self.active_page
        self.active_page = page_name
        self._refresh_navigation(animate=animate)
        for widget in self.page_container.winfo_children():
            if widget is self.models_frame:
                widget.grid_remove()
            else:
                widget.destroy()

        if page_name == "Загрузка моделей":
            if self.models_frame is None:
                self.models_frame = ModelsFrame(self.page_container, self)
            self.models_frame.grid(row=0, column=0, rowspan=3, sticky="nsew")
        elif page_name == "Настройки":
            SettingsFrame(self.page_container, self).grid(
                row=0, column=0, rowspan=3, sticky="nsew"
            )
        else:
            ctk.CTkLabel(
                self.page_container,
                text=page_name,
                text_color=self.TEXT,
                font=ctk.CTkFont(size=30, weight="bold"),
            ).grid(row=0, column=0, sticky="w")
            ctk.CTkLabel(
                self.page_container,
                text=self.SUBTITLES[page_name],
                text_color=self.MUTED,
                font=ctk.CTkFont(size=15),
            ).grid(row=1, column=0, pady=(4, 22), sticky="w")

            body = ctk.CTkFrame(self.page_container, fg_color="transparent")
            body.grid(row=2, column=0, sticky="nsew")
            body.grid_columnconfigure(0, weight=1)
            body.grid_rowconfigure(0, weight=1)

            render_page = {
                "Главная": self._render_home,
                "Галерея": self._render_gallery,
            }
            render_page[page_name](body)
        self.discord_presence.set_page(page_name)

    def _on_close(self):
        if self._nav_highlight_job is not None:
            self.after_cancel(self._nav_highlight_job)
        if self._package_poll_job is not None:
            self.after_cancel(self._package_poll_job)
        if self.install_dialog is not None and self.install_dialog.winfo_exists():
            if self.install_dialog.installing:
                threading.Thread(target=self.install_dialog.installer.cancel, daemon=True).start()
        if self.models_frame is not None:
            self.models_frame.stop()
        self.discord_presence.close()
        self.destroy()

    def _card(self, parent):
        return ctk.CTkFrame(
            parent,
            corner_radius=16,
            fg_color=self.CARD,
        )

    def _render_home(self, body):
        packages = installed_packages()
        body.grid_rowconfigure(0, weight=0)
        body.grid_rowconfigure(1, weight=1)

        overview = ctk.CTkFrame(body, fg_color="transparent")
        overview.grid(row=0, column=0, pady=(0, 18), sticky="ew")
        overview.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            overview, text="ВАША БИБЛИОТЕКА", text_color=self.ACCENT,
            font=ctk.CTkFont(size=12, weight="bold"),
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            overview, text="Пакеты и окружения", text_color=self.TEXT,
            font=ctk.CTkFont(size=21, weight="bold"),
        ).grid(row=1, column=0, pady=(3, 0), sticky="w")
        ctk.CTkLabel(
            overview, text=f"Пакетов: {len(packages)}", text_color=self.MUTED,
            fg_color=self.CARD, corner_radius=9,
            font=ctk.CTkFont(size=12), width=112, height=30,
        ).grid(row=0, column=1, rowspan=2, sticky="e")

        board = ctk.CTkFrame(body, fg_color="transparent")
        board.grid(row=1, column=0, sticky="nsew")
        board.grid_columnconfigure(0, weight=3)
        board.grid_columnconfigure(1, weight=2)
        board.grid_rowconfigure(0, weight=1)

        library = self._card(board)
        library.grid(row=0, column=0, padx=(0, 16), sticky="nsew")
        library.grid_columnconfigure(0, weight=1)
        library.grid_rowconfigure(1, weight=1)
        ctk.CTkLabel(
            library, text="Установленные пакеты", text_color=self.TEXT,
            font=ctk.CTkFont(size=18, weight="bold"),
        ).grid(row=0, column=0, padx=24, pady=(22, 0), sticky="w")

        if packages:
            list_frame = ctk.CTkFrame(library, fg_color="transparent")
            list_frame.grid(row=1, column=0, padx=24, pady=20, sticky="nsew")
            list_frame.grid_columnconfigure(0, weight=1)
            for row, (name, path) in enumerate(packages):
                self._render_package_card(list_frame, row, name, path)
        else:
            empty = ctk.CTkFrame(library, fg_color="transparent")
            empty.grid(row=1, column=0, padx=24)
            ctk.CTkLabel(
                empty, text="◈", text_color=self.ACCENT, font=ctk.CTkFont(size=48)
            ).pack()
            ctk.CTkLabel(
                empty, text="Библиотека пока пуста", text_color=self.TEXT,
                font=ctk.CTkFont(size=18, weight="bold"),
            ).pack(pady=(8, 4))
            ctk.CTkLabel(
                empty, text="Добавьте первый пакет, чтобы начать работу.",
                text_color=self.MUTED, font=ctk.CTkFont(size=13),
                wraplength=310,
            ).pack()

        add_card = ctk.CTkFrame(board, corner_radius=16, fg_color="#20354A")
        add_card.grid(row=0, column=1, sticky="nsew")
        add_card.grid_columnconfigure(0, weight=1)
        add_card.grid_rowconfigure(3, weight=1)
        ctk.CTkLabel(
            add_card, text="＋", width=48, height=48, corner_radius=12,
            fg_color=self.ACCENT, text_color=self.TEXT,
            font=ctk.CTkFont(size=28),
        ).grid(row=0, column=0, padx=20, pady=(24, 20), sticky="w")
        ctk.CTkLabel(
            add_card, text="Новый пакет", text_color=self.TEXT,
            font=ctk.CTkFont(size=21, weight="bold"),
        ).grid(row=1, column=0, padx=20, sticky="w")
        ctk.CTkLabel(
            add_card,
            text="Установите WebUI в отдельное окружение. Flow покажет каждый шаг установки.",
            text_color=self.MUTED, wraplength=190, justify="left",
            font=ctk.CTkFont(size=13),
        ).grid(row=2, column=0, padx=20, pady=(8, 0), sticky="nw")
        ctk.CTkButton(
            add_card, text="Добавить пакет  →", height=46,
            corner_radius=10,
            fg_color=self.ACCENT,
            hover_color=self.ACCENT_HOVER,
            font=ctk.CTkFont(size=15, weight="bold"),
            command=self._open_install_dialog,
        ).grid(row=4, column=0, padx=20, pady=(16, 8), sticky="ew")
        self.install_status = ctk.CTkLabel(
            add_card, text="", text_color=self.MUTED,
            font=ctk.CTkFont(size=12), wraplength=190,
        )
        self.install_status.grid(row=5, column=0, padx=20, pady=(0, 18), sticky="w")

    def _render_package_card(self, parent, row, name, path):
        item = ctk.CTkFrame(parent, fg_color=self.SURFACE, corner_radius=12)
        item.grid(row=row, column=0, pady=(0, 10), sticky="ew")
        item.grid_columnconfigure(1, weight=1)
        ctk.CTkLabel(
            item, text="A", width=42, height=42, corner_radius=10,
            fg_color="#204F73", text_color=self.TEXT,
            font=ctk.CTkFont(size=19, weight="bold"),
        ).grid(row=0, column=0, rowspan=2, padx=(16, 12), pady=(16, 0))
        ctk.CTkLabel(
            item, text=name, text_color=self.TEXT,
            font=ctk.CTkFont(size=16, weight="bold"),
        ).grid(row=0, column=1, pady=(16, 1), sticky="w")
        ctk.CTkLabel(
            item, text=f"{package_version()}  ·  Stable Diffusion",
            text_color=self.MUTED, font=ctk.CTkFont(size=12),
        ).grid(row=1, column=1, sticky="w")
        self.package_badge = ctk.CTkLabel(
            item, text="●  Готов", text_color=self.SUCCESS,
            fg_color="#24443D", corner_radius=8,
            font=ctk.CTkFont(size=11, weight="bold"), width=82, height=26,
        )
        self.package_badge.grid(row=2, column=1, pady=(8, 0), sticky="w")
        ctk.CTkLabel(
            item, text=str(path), text_color=self.MUTED,
            font=ctk.CTkFont(size=11), anchor="w", wraplength=270,
        ).grid(row=3, column=0, columnspan=2, padx=16, pady=(12, 14), sticky="ew")
        actions = ctk.CTkFrame(item, fg_color="transparent")
        actions.grid(row=4, column=0, columnspan=2, padx=16, pady=(0, 16), sticky="ew")
        actions.grid_columnconfigure(0, weight=1)
        actions.grid_columnconfigure(1, weight=1)
        self.package_start_button = ctk.CTkButton(
            actions, text="Запустить", height=38, corner_radius=9,
            fg_color=self.ACCENT, hover_color=self.ACCENT_HOVER,
            command=self._launch_webui,
        )
        self.package_start_button.grid(row=0, column=0, padx=(0, 6), sticky="ew")
        ctk.CTkButton(
            actions, text="Настроить", height=38, corner_radius=9,
            fg_color=self.HOVER, hover_color=self.BORDER,
            command=self._open_package_settings,
        ).grid(row=0, column=1, padx=(6, 0), sticky="ew")
        self._update_package_widgets()

    def _open_install_dialog(self):
        if self.install_dialog is not None and self.install_dialog.winfo_exists():
            self.install_dialog.lift()
            self.install_dialog.focus()
            return
        self.install_dialog = InstallDialog(self, self._refresh_installed_packages)

    def _refresh_installed_packages(self):
        if self.active_page == "Главная":
            self.show_page("Главная")

    def _launch_webui(self):
        if self.webui_process is not None and self.webui_process.poll() is None:
            return
        try:
            self.webui_process = launch_webui()
        except (InstallationError, OSError) as exc:
            self.package_error = str(exc)
            self.install_status.configure(text=f"Ошибка запуска: {exc}")
            self._update_package_widgets()
        else:
            self.package_error = None
            self.install_status.configure(text="WebUI запускается в отдельном окне консоли")
            self._update_package_widgets()

    def _open_package_settings(self):
        if (self.package_settings_dialog is not None
                and self.package_settings_dialog.winfo_exists()):
            self.package_settings_dialog.lift()
            return
        packages = installed_packages()
        if not packages:
            self.install_status.configure(text="Пакет ещё не установлен")
            return
        name, path = packages[0]
        window = ctk.CTkToplevel(self, fg_color=self.BACKGROUND)
        self.package_settings_dialog = window
        window.title("Параметры пакета · Flow")
        window.geometry("560x310")
        window.minsize(500, 300)
        window.transient(self)
        window.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            window, text="Параметры пакета", text_color=self.TEXT,
            font=ctk.CTkFont(size=23, weight="bold"),
        ).grid(row=0, column=0, padx=24, pady=(22, 14), sticky="w")
        card = self._card(window)
        card.grid(row=1, column=0, padx=24, sticky="ew")
        card.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            card, text=name, text_color=self.TEXT,
            font=ctk.CTkFont(size=17, weight="bold"),
        ).grid(row=0, column=0, padx=18, pady=(16, 2), sticky="w")
        ctk.CTkLabel(
            card, text=f"Версия: {package_version()}", text_color=self.MUTED,
        ).grid(row=1, column=0, padx=18, sticky="w")
        ctk.CTkLabel(
            card, text=str(path), text_color=self.MUTED,
            wraplength=460, justify="left",
        ).grid(row=2, column=0, padx=18, pady=(8, 16), sticky="w")
        status = ctk.CTkLabel(window, text="", text_color=self.MUTED)
        status.grid(row=2, column=0, padx=24, pady=(8, 0), sticky="w")
        actions = ctk.CTkFrame(window, fg_color="transparent")
        actions.grid(row=3, column=0, padx=24, pady=(8, 20), sticky="ew")
        actions.grid_columnconfigure(0, weight=1)
        ctk.CTkButton(
            actions, text="Открыть настройки WebUI", height=40,
            fg_color=self.ACCENT, hover_color=self.ACCENT_HOVER,
            command=lambda: self._edit_package_settings(status),
        ).grid(row=0, column=0, padx=(0, 8), sticky="ew")
        ctk.CTkButton(
            actions, text="Закрыть", width=100, height=40,
            fg_color=self.HOVER, hover_color=self.BORDER,
            command=window.destroy,
        ).grid(row=0, column=1)
        window.after(100, window.grab_set)

    @staticmethod
    def _edit_package_settings(status):
        try:
            open_package_settings()
        except (InstallationError, OSError) as exc:
            status.configure(text=f"Не удалось открыть файл: {exc}")
        else:
            status.configure(text="Файл настроек открыт во внешнем редакторе")

    def _update_package_widgets(self):
        running = self.webui_process is not None and self.webui_process.poll() is None
        if self.package_badge is not None and self.package_badge.winfo_exists():
            if running:
                text, color, background = "●  Запущен", self.SUCCESS, "#24443D"
            elif self.package_error:
                text, color, background = "●  Ошибка", "#F19999", "#4B3034"
            else:
                text, color, background = "●  Готов", "#9CC9EA", "#27445C"
            self.package_badge.configure(text=text, text_color=color, fg_color=background)
        if self.package_start_button is not None and self.package_start_button.winfo_exists():
            self.package_start_button.configure(state="disabled" if running else "normal")

    def _sync_package_status(self):
        self._package_poll_job = None
        if self.webui_process is not None and self.webui_process.poll() is not None:
            if self.webui_process.returncode:
                self.package_error = "WebUI завершился с ошибкой"
            self.webui_process = None
        self._update_package_widgets()
        self._package_poll_job = self.after(1500, self._sync_package_status)

    def _render_gallery(self, body):
        card = self._card(body)
        card.grid(row=0, column=0, sticky="nsew")
        for column in range(3):
            card.grid_columnconfigure(column, weight=1)
        card.grid_rowconfigure(2, weight=1)

        ctk.CTkLabel(
            card,
            text="Ваша галерея",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=19, weight="bold"),
        ).grid(row=0, column=0, columnspan=3, padx=24, pady=(22, 0), sticky="w")

        for column in range(3):
            tile = ctk.CTkFrame(
                card, height=154, corner_radius=12, fg_color=self.SURFACE,
            )
            tile.grid(
                row=1, column=column,
                padx=(24 if column == 0 else 8, 24 if column == 2 else 8),
                pady=(24, 0), sticky="ew",
            )
            tile.grid_propagate(False)
            ctk.CTkLabel(
                tile, text="▧", text_color="#566576", font=ctk.CTkFont(size=36),
            ).place(relx=0.5, rely=0.5, anchor="center")

        empty = ctk.CTkFrame(card, fg_color="transparent")
        empty.grid(row=2, column=0, columnspan=3)
        ctk.CTkLabel(
            empty, text="Здесь пока нет изображений", text_color=self.TEXT,
            font=ctk.CTkFont(size=18, weight="bold"),
        ).pack(pady=(0, 5))
        ctk.CTkLabel(
            empty, text="После генерации изображения появятся в этой галерее.",
            text_color=self.MUTED, font=ctk.CTkFont(size=13),
        ).pack()

if __name__ == "__main__":
    FlowApp().mainloop()
