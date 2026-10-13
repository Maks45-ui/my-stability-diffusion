from pathlib import Path
import sys
from tkinter import TclError

import customtkinter as ctk
from PIL import Image, ImageTk

from discord_presence import DiscordPresence


ctk.set_appearance_mode("dark")
ctk.set_default_color_theme("blue")


class FlowApp(ctk.CTk):
    BACKGROUND = "#1E1E1E"
    SIDEBAR = "#252526"
    CARD = "#2B2B2D"
    BORDER = "#39393D"
    HOVER = "#34363A"
    ACCENT = "#2B82C5"
    ACCENT_HOVER = "#1F6AA5"
    TEXT = "#F5F6F8"
    MUTED = "#A9ADB5"

    EXPANDED_WIDTH = 220
    COLLAPSED_WIDTH = 70
    NAV_ITEMS = (
        ("Главная", "⌂"),
        ("Галерея", "▦"),
        ("Загрузка моделей", "↓"),
        ("Настройки", "⚙"),
    )
    SUBTITLES = {
        "Главная": "Управляйте пакетами и инструментами в одном месте.",
        "Галерея": "Здесь будут собраны ваши сгенерированные изображения.",
        "Загрузка моделей": "Каталог и загрузки моделей с Civitai.",
        "Настройки": "Параметры приложения Flow.",
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

        self.logo_image = None
        self.window_icon = None
        self._load_logo()

        self.sidebar_expanded = True
        self.active_page = "Главная"
        self.nav_buttons = {}
        self.discord_presence = DiscordPresence()

        self._build_sidebar()
        self._build_main_area()
        self.show_page("Главная")
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _load_logo(self):
        base_dir = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))
        for logo_path in (base_dir / "assets" / "logo.png", base_dir / "logo.png"):
            try:
                with Image.open(logo_path) as source:
                    logo = source.convert("RGBA")
                self.logo_image = ctk.CTkImage(
                    light_image=logo, dark_image=logo, size=(54, 54)
                )
                self.window_icon = ImageTk.PhotoImage(logo)
                self.after(250, self._set_window_icon)
                return
            except (OSError, ValueError, TclError):
                continue

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

        for row, (name, symbol) in enumerate(self.NAV_ITEMS[:3], start=1):
            self._add_nav_button(name, symbol, row)
        name, symbol = self.NAV_ITEMS[3]
        self._add_nav_button(name, symbol, 5)

    def _add_nav_button(self, name, symbol, row):
        item = ctk.CTkFrame(
            self.sidebar,
            width=46,
            height=44,
            corner_radius=10,
            fg_color=self.SIDEBAR,
        )
        item.grid(row=row, column=0, padx=12, pady=4, sticky="ew")
        item.grid_propagate(False)
        item.grid_columnconfigure(1, weight=1)

        icon_button = ctk.CTkButton(
            item,
            text=symbol,
            width=46,
            height=44,
            corner_radius=10,
            fg_color="transparent",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=18),
            command=lambda page=name: self.show_page(page),
        )
        icon_button.grid(row=0, column=0)

        label_button = ctk.CTkButton(
            item,
            text=name,
            height=44,
            corner_radius=10,
            fg_color="transparent",
            text_color=self.TEXT,
            anchor="w",
            font=ctk.CTkFont(size=15),
            command=lambda page=name: self.show_page(page),
        )
        label_button.grid(row=0, column=1, sticky="ew")
        self.nav_buttons[name] = (item, icon_button, label_button)

    def _build_main_area(self):
        main = ctk.CTkFrame(self, corner_radius=0, fg_color=self.BACKGROUND)
        main.grid(row=0, column=1, sticky="nsew")
        main.grid_columnconfigure(0, weight=1)
        main.grid_rowconfigure(2, weight=1)

        header = ctk.CTkFrame(main, height=96, corner_radius=0, fg_color=self.BACKGROUND)
        header.grid(row=0, column=0, sticky="ew")
        header.grid_propagate(False)
        header.grid_columnconfigure(1, weight=1)

        if self.logo_image is not None:
            logo_label = ctk.CTkLabel(header, image=self.logo_image, text="")
        else:
            logo_label = ctk.CTkLabel(
                header,
                text="F",
                width=54,
                height=54,
                corner_radius=13,
                fg_color=self.ACCENT,
                text_color=self.TEXT,
                font=ctk.CTkFont(size=30, weight="bold"),
            )
        logo_label.grid(row=0, column=0, rowspan=2, padx=(32, 14), pady=21)

        ctk.CTkLabel(
            header,
            text="Flow",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=25, weight="bold"),
        ).grid(row=0, column=1, sticky="sw", pady=(18, 0))
        ctk.CTkLabel(
            header,
            text="Творчество под вашим контролем",
            text_color=self.MUTED,
            font=ctk.CTkFont(size=13),
        ).grid(row=1, column=1, sticky="nw", pady=(0, 18))

        ctk.CTkFrame(main, height=1, corner_radius=0, fg_color=self.BORDER).grid(
            row=1, column=0, sticky="ew"
        )

        self.page_container = ctk.CTkFrame(main, fg_color="transparent")
        self.page_container.grid(row=2, column=0, padx=32, pady=(25, 28), sticky="nsew")
        self.page_container.grid_columnconfigure(0, weight=1)
        self.page_container.grid_rowconfigure(2, weight=1)

    def toggle_sidebar(self):
        self.sidebar_expanded = not self.sidebar_expanded
        if self.sidebar_expanded:
            self.sidebar.configure(width=self.EXPANDED_WIDTH)
            for _, _, label_button in self.nav_buttons.values():
                label_button.grid()
        else:
            for _, _, label_button in self.nav_buttons.values():
                label_button.grid_remove()
            self.sidebar.configure(width=self.COLLAPSED_WIDTH)

    def _refresh_navigation(self):
        for name, (item, icon_button, label_button) in self.nav_buttons.items():
            selected = name == self.active_page
            item.configure(fg_color=self.ACCENT if selected else self.SIDEBAR)
            for button in (icon_button, label_button):
                button.configure(
                    hover_color=self.ACCENT_HOVER if selected else self.HOVER,
                    text_color=self.TEXT,
                )

    def show_page(self, page_name):
        self.active_page = page_name
        self._refresh_navigation()
        for widget in self.page_container.winfo_children():
            widget.destroy()

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
            "Загрузка моделей": self._render_models,
            "Настройки": self._render_settings,
        }
        render_page[page_name](body)
        self.discord_presence.set_page(page_name)

    def _on_close(self):
        self.discord_presence.close()
        self.destroy()

    def _card(self, parent):
        return ctk.CTkFrame(
            parent,
            corner_radius=18,
            fg_color=self.CARD,
            border_width=1,
            border_color=self.BORDER,
        )

    def _render_home(self, body):
        card = self._card(body)
        card.grid(row=0, column=0, sticky="nsew")
        card.grid_columnconfigure(0, weight=1)
        card.grid_rowconfigure(1, weight=1)

        ctk.CTkLabel(
            card,
            text="Установленные пакеты",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=20, weight="bold"),
        ).grid(row=0, column=0, padx=28, pady=(24, 0), sticky="w")

        empty = ctk.CTkFrame(card, fg_color="transparent")
        empty.grid(row=1, column=0)
        ctk.CTkLabel(
            empty, text="◈", text_color=self.ACCENT, font=ctk.CTkFont(size=50)
        ).pack()
        ctk.CTkLabel(
            empty,
            text="Пакетов пока нет",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=20, weight="bold"),
        ).pack(pady=(8, 4))
        ctk.CTkLabel(
            empty,
            text="Когда вы добавите первый пакет, он появится здесь.",
            text_color=self.MUTED,
            font=ctk.CTkFont(size=14),
        ).pack()

        ctk.CTkButton(
            body,
            text="＋  Добавить пакет",
            width=270,
            height=54,
            corner_radius=14,
            fg_color=self.ACCENT,
            hover_color=self.ACCENT_HOVER,
            font=ctk.CTkFont(size=17, weight="bold"),
            command=self._show_install_status,
        ).grid(row=1, column=0, pady=(22, 4))
        self.install_status = ctk.CTkLabel(
            body, text="", text_color=self.MUTED, font=ctk.CTkFont(size=13)
        )
        self.install_status.grid(row=2, column=0)

    def _show_install_status(self):
        self.install_status.configure(text="Установка пока недоступна")

    def _render_gallery(self, body):
        card = self._card(body)
        card.grid(row=0, column=0, sticky="nsew")
        for column in range(3):
            card.grid_columnconfigure(column, weight=1)

        ctk.CTkLabel(
            card,
            text="Ваши изображения",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=20, weight="bold"),
        ).grid(row=0, column=0, columnspan=3, padx=28, pady=(24, 0), sticky="w")

        for column in range(3):
            tile = ctk.CTkFrame(
                card, height=160, corner_radius=13, fg_color="#353537"
            )
            tile.grid(
                row=1, column=column, padx=(28 if column == 0 else 8, 28 if column == 2 else 8),
                pady=(28, 0), sticky="ew"
            )
            tile.grid_propagate(False)
            ctk.CTkLabel(
                tile, text="▧", text_color="#62666D", font=ctk.CTkFont(size=40)
            ).place(relx=0.5, rely=0.5, anchor="center")

        ctk.CTkLabel(
            card,
            text="Здесь пока нет изображений",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=18, weight="bold"),
        ).grid(row=2, column=0, columnspan=3, pady=(30, 4))
        ctk.CTkLabel(
            card,
            text="Галерея будет показывать результаты генерации.",
            text_color=self.MUTED,
            font=ctk.CTkFont(size=14),
        ).grid(row=3, column=0, columnspan=3, pady=(0, 28))

    def _render_models(self, body):
        body.grid_columnconfigure(1, weight=1)
        self._feature_card(
            body, 0, "⌕", "Каталог Civitai",
            "Поиск и просмотр моделей появятся здесь."
        )
        self._feature_card(
            body, 1, "↓", "Загрузки",
            "Здесь будет отображаться очередь скачивания моделей."
        )

    def _feature_card(self, parent, column, symbol, title, description):
        card = self._card(parent)
        card.grid(
            row=0, column=column,
            padx=(0, 8) if column == 0 else (8, 0),
            sticky="nsew"
        )
        ctk.CTkLabel(
            card, text=symbol, text_color=self.ACCENT, font=ctk.CTkFont(size=42)
        ).pack(anchor="w", padx=26, pady=(28, 18))
        ctk.CTkLabel(
            card, text=title, text_color=self.TEXT,
            font=ctk.CTkFont(size=19, weight="bold")
        ).pack(anchor="w", padx=26)
        ctk.CTkLabel(
            card, text=description, text_color=self.MUTED,
            font=ctk.CTkFont(size=14), justify="left", wraplength=250
        ).pack(anchor="w", padx=26, pady=(10, 26))

    def _render_settings(self, body):
        card = self._card(body)
        card.grid(row=0, column=0, sticky="nsew")
        ctk.CTkLabel(
            card,
            text="Настройки Flow",
            text_color=self.TEXT,
            font=ctk.CTkFont(size=20, weight="bold"),
        ).pack(anchor="w", padx=28, pady=(26, 10))
        ctk.CTkLabel(
            card,
            text="Параметры приложения будут доступны здесь.",
            text_color=self.MUTED,
            font=ctk.CTkFont(size=14),
        ).pack(anchor="w", padx=28)


if __name__ == "__main__":
    FlowApp().mainloop()
