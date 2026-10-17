import type { ResolvedTheme, Theme, ThemeState } from "./types";

interface ThemeOption {
  id: Theme;
  label: string;
  description: string;
  group: "standard" | "extra";
}

const STORAGE_KEY = "flow.theme";
const OPTIONS: readonly ThemeOption[] = [
  { id: "light", label: "Светлая", description: "Чистый нейтральный фон", group: "standard" },
  { id: "dark", label: "Тёмная", description: "Графит и холодный синий", group: "standard" },
  { id: "system", label: "Системная", description: "Следует за Windows", group: "standard" },
  { id: "zakaz", label: "Заказ", description: "Уголь, янтарь и бордо", group: "extra" },
  { id: "crimson-moon", label: "Багровая Луна", description: "Уголь и приглушённый красный", group: "extra" },
  { id: "blueviolet-dusk", label: "Синелетовые сумерки", description: "Холодный синий и лаванда", group: "extra" }
];

function savedTheme(): Theme {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    const option = OPTIONS.find((item) => item.id === stored);
    return option?.id ?? "dark";
  } catch {
    return "dark";
  }
}

function makeElement<K extends keyof HTMLElementTagNameMap>(tag: K, className: string): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.className = className;
  return element;
}

/** Keeps the selected palette and its effective system color scheme in sync. */
export class ThemeController {
  private readonly systemDark = window.matchMedia("(prefers-color-scheme: dark)");
  private readonly state: ThemeState = { selected: savedTheme(), resolved: "dark" };
  private readonly buttons = new Map<Theme, HTMLButtonElement>();

  constructor(
    private readonly standard: HTMLElement,
    private readonly extra: HTMLElement,
    private readonly stateLabel: HTMLElement,
    private readonly headerLabel: HTMLElement,
    private readonly headerIcon: HTMLElement
  ) {
    this.renderOptions();
    this.apply();
    this.systemDark.addEventListener("change", () => {
      if (this.state.selected === "system") this.apply();
    });
  }

  private renderOptions(): void {
    for (const option of OPTIONS) {
      const button = makeElement("button", "theme-option glass-surface");
      button.type = "button";
      button.dataset.themeOption = option.id;
      button.setAttribute("aria-pressed", "false");
      button.setAttribute("aria-label", `${option.label}: ${option.description}`);

      const preview = makeElement("span", "theme-preview");
      preview.setAttribute("aria-hidden", "true");
      preview.append(makeElement("span", "theme-preview-panel"), makeElement("span", "theme-preview-accent"));
      const copy = makeElement("span", "theme-option-copy");
      const title = makeElement("strong", "theme-option-name");
      title.textContent = option.label;
      const description = makeElement("small", "theme-option-description");
      description.textContent = option.description;
      copy.append(title, description);
      button.append(preview, copy);
      button.addEventListener("click", () => this.select(option.id));
      (option.group === "standard" ? this.standard : this.extra).append(button);
      this.buttons.set(option.id, button);
    }
  }

  private resolve(): ResolvedTheme {
    if (this.state.selected === "system") return this.systemDark.matches ? "dark" : "light";
    return this.state.selected;
  }

  private apply(): void {
    this.state.resolved = this.resolve();
    document.documentElement.dataset.theme = this.state.resolved;
    const name = OPTIONS.find((option) => option.id === this.state.selected)?.label ?? "Тёмная";
    this.stateLabel.textContent = this.state.selected === "system"
      ? `${name} · ${this.state.resolved === "dark" ? "тёмная" : "светлая"}` : name;
    this.headerLabel.textContent = name;
    this.headerIcon.textContent = this.state.resolved === "light" ? "☀" : "☾";
    for (const [id, button] of this.buttons) {
      const selected = id === this.state.selected;
      button.classList.toggle("selected", selected);
      button.setAttribute("aria-pressed", String(selected));
    }
  }

  select(theme: Theme): void {
    this.state.selected = theme;
    this.apply();
    try { localStorage.setItem(STORAGE_KEY, theme); } catch { /* The choice still applies to this window. */ }
  }
}
