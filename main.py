import customtkinter as ctk


ctk.set_default_color_theme("blue")


class StabilityMatrixApp(ctk.CTk):
    PAGES = {
        "Главная": "Добро пожаловать! Здесь появится обзор установленных приложений и моделей.",
        "Установка приложений": "Здесь можно будет устанавливать приложения для запуска нейросетей.",
        "Настройки": "Здесь появятся настройки окружений, загрузок и внешнего вида.",
    }

    def __init__(self):
        super().__init__()
        self.title("My Stability Matrix")
        self.geometry("960x600")
        self.minsize(760, 480)
        self.grid_rowconfigure(0, weight=1)
        self.grid_columnconfigure(1, weight=1)

        sidebar = ctk.CTkFrame(self, width=230, corner_radius=0)
        sidebar.grid(row=0, column=0, sticky="nsew")
        sidebar.grid_propagate(False)
        sidebar.grid_columnconfigure(0, weight=1)

        ctk.CTkLabel(
            sidebar, text="My Stability Matrix", font=ctk.CTkFont(size=19, weight="bold")
        ).grid(row=0, column=0, padx=20, pady=(28, 32), sticky="w")

        self.nav_buttons = {}
        for row, page_name in enumerate(self.PAGES, start=1):
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
    StabilityMatrixApp().mainloop()

