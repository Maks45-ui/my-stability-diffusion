from pathlib import Path
import sys

import customtkinter as ctk
from PIL import Image, ImageTk


ctk.set_default_color_theme("blue")


class FlowApp(ctk.CTk):
    PAGES = {
        "Главная": "Добро пожаловать! Здесь появится обзор установленных приложений и моделей.",
        "Установка приложений": "Здесь можно будет устанавливать приложения для запуска нейросетей.",
        "Настройки": "Здесь появятся настройки окружений, загрузок и внешнего вида.",
    }

    def __init__(self):
        super().__init__()
        self.title("Flow")
        self.geometry("960x600")
        self.minsize(760, 480)
        self.grid_rowconfigure(0, weight=1)
        self.grid_columnconfigure(1, weight=1)

        assets_dir = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent)) / "assets"
        with Image.open(assets_dir / "logo.png") as image:
            logo = image.convert("RGBA")
        self.logo_image = ctk.CTkImage(light_image=logo, dark_image=logo, size=(72, 72))
        self.window_icon = ImageTk.PhotoImage(logo)
        self.after(250, lambda: self.wm_iconphoto(True, self.window_icon))

        sidebar = ctk.CTkFrame(self, width=230, corner_radius=0)
        sidebar.grid(row=0, column=0, sticky="nsew")
        sidebar.grid_propagate(False)
        sidebar.grid_columnconfigure(0, weight=1)

        ctk.CTkLabel(sidebar, image=self.logo_image, text="").grid(
            row=0, column=0, padx=20, pady=(24, 8), sticky="w"
        )
        ctk.CTkLabel(sidebar, text="Flow", font=ctk.CTkFont(size=22, weight="bold")).grid(
            row=1, column=0, padx=20, pady=(0, 28), sticky="w"
        )

        self.nav_buttons = {}
        for row, page_name in enumerate(self.PAGES, start=2):
            button = ctk.CTkButton(
                sidebar,
                text=page_name,
                anchor="w",
                fg_color="transparent",
                text_color=("gray10", "gray90"),
                hover_color=("gray80", "gray30"),
                command=lambda name=page_name: self.show_page(name),
            )
            button.grid(row=row, column=0, padx=14, pady=4, sticky="ew")
            self.nav_buttons[page_name] = button

        self.content = ctk.CTkFrame(self, fg_color="transparent")
        self.content.grid(row=0, column=1, padx=36, pady=32, sticky="nsew")
        self.content.grid_columnconfigure(0, weight=1)
        self.show_page("Главная")

    def show_page(self, page_name):
        for widget in self.content.winfo_children():
            widget.destroy()

        for name, button in self.nav_buttons.items():
            if name == page_name:
                button.configure(fg_color=("#3B8ED0", "#1F6AA5"), text_color="white")
            else:
                button.configure(fg_color="transparent", text_color=("gray10", "gray90"))

        ctk.CTkLabel(
            self.content, text=page_name, font=ctk.CTkFont(size=28, weight="bold")
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            self.content,
            text=self.PAGES[page_name],
            font=ctk.CTkFont(size=15),
            wraplength=600,
            justify="left",
        ).grid(row=1, column=0, pady=(16, 0), sticky="w")


if __name__ == "__main__":
    FlowApp().mainloop()
