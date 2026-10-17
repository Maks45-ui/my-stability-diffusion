import { startSplashScreen } from "./splash";
import type { FlowAPI, ModelFile, StarterModel, TaskEvent } from "./types";

type StarterKey = "indigo" | "wai" | "realistic";

function required<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Элемент ${id} не найден.`);
  return node as T;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Не удалось выполнить действие.";
}

/** Keeps the main page inaccessible until both first-run steps are complete. */
export class FirstRunController {
  private readonly dialog = required<HTMLDialogElement>("first-run-dialog");
  private readonly title = required<HTMLElement>("first-run-title");
  private readonly intro = required<HTMLElement>("first-run-intro");
  private readonly keyStep = required<HTMLElement>("first-run-key-step");
  private readonly modelStep = required<HTMLElement>("first-run-model-step");
  private readonly modelError = required<HTMLElement>("first-run-model-error");
  private readonly form = required<HTMLFormElement>("first-run-form");
  private readonly input = required<HTMLInputElement>("first-run-token");
  private readonly status = required<HTMLElement>("first-run-status");
  private readonly save = required<HTMLButtonElement>("first-run-save");
  private readonly skip = required<HTMLButtonElement>("first-run-skip");
  private readonly modelSkip = required<HTMLButtonElement>("first-run-model-skip");
  private readonly site = required<HTMLAnchorElement>("first-run-site");
  private readonly models = new Map<StarterKey, StarterModel>();
  private activeTaskId: string | null = null;
  private activeKey: StarterKey | null = null;
  private busy = false;
  private modelsLoaded = false;

  constructor(
    private readonly api: FlowAPI,
    private readonly app: HTMLElement,
    private readonly enqueue: (file: ModelFile, type: string, name: string) => string | null,
    private readonly onComplete: () => void
  ) {
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.saveKey(this.input.value.trim());
    });
    this.skip.addEventListener("click", () => void this.saveKey(null));
    this.modelSkip.addEventListener("click", () => void this.finish());
    this.dialog.addEventListener("cancel", (event) => event.preventDefault());
    this.site.addEventListener("click", (event) => {
      event.preventDefault();
      void this.api.openCivitaiSite().catch((error: unknown) => this.showError(errorMessage(error)));
    });
    this.cards().forEach((card) => {
      const key = card.dataset.starter as StarterKey;
      card.querySelector<HTMLButtonElement>(".first-run-download")?.addEventListener("click", () => {
        if (this.models.has(key)) void this.selectModel(key);
        else void this.loadModel(key);
      });
    });
  }

  async start(): Promise<void> {
    this.app.inert = true;
    // Begin the disk read during the welcome animation and handle rejection immediately.
    const status = this.api.getOnboardingStage()
      .then((stage) => ({ stage, error: "" }))
      .catch((error: unknown) => ({ stage: 0, error: errorMessage(error) }));
    await startSplashScreen();
    const result = await status;
    if (result.stage >= 2) { this.app.inert = false; return; }
    this.dialog.showModal();
    if (result.stage === 1) this.showModelStep();
    if (result.error) this.showError(`Не удалось проверить первый запуск: ${result.error}`);
    if (result.stage === 0) this.input.focus();
  }

  onTask(event: TaskEvent): void {
    if (event.kind !== "download" || !this.activeTaskId || event.taskId !== this.activeTaskId || !this.activeKey) return;
    const card = this.card(this.activeKey);
    const progress = card.querySelector<HTMLElement>(".first-run-model-progress");
    const fill = card.querySelector<HTMLElement>(".first-run-model-track > div");
    const percent = card.querySelector<HTMLElement>(".first-run-model-percent");
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    if (progress) progress.hidden = false;
    if (event.percent !== undefined) {
      if (fill) fill.style.width = `${event.percent}%`;
      if (percent) percent.textContent = `${event.percent}%`;
    }
    if (status && event.phase === "dependencies") {
      const speed = event.bytesPerSecond ? ` · ${(event.bytesPerSecond / 1048576).toFixed(1)} МБ/с` : "";
      status.textContent = `Скачивается${speed}`;
    }
    if (event.phase === "done") {
      if (status) status.textContent = "Модель загружена. Открываем Flow…";
      this.activeTaskId = null;
      void this.finish();
    } else if (["error", "cancelled", "paused"].includes(event.phase)) {
      if (status) status.textContent = event.phase === "paused" ? "Загрузка приостановлена. Продолжите в каталоге." : event.message;
      this.activeTaskId = null;
      this.modelError.textContent = "Можно выбрать другую модель или начать работу без загрузки.";
      this.enableModelButtons(true);
    }
  }

  private cards(): HTMLElement[] { return [...document.querySelectorAll<HTMLElement>(".first-run-model-card")]; }

  private card(key: StarterKey): HTMLElement {
    const card = this.cards().find((item) => item.dataset.starter === key);
    if (!card) throw new Error(`Карточка ${key} не найдена.`);
    return card;
  }

  private showError(message: string): void {
    this.status.textContent = message;
    this.status.classList.add("error");
  }

  private async saveKey(token: string | null): Promise<void> {
    if (this.busy) return;
    if (token !== null && token.length === 0) {
      this.showError("Введите API-ключ или нажмите «Пропустить».");
      this.input.focus();
      return;
    }
    this.busy = true;
    this.save.disabled = true;
    this.skip.disabled = true;
    this.input.disabled = true;
    this.status.classList.remove("error");
    this.status.textContent = "Сохранение…";
    try {
      await this.api.saveOnboardingToken(token);
      this.input.value = "";
      this.showModelStep();
    } catch (error) {
      this.showError(errorMessage(error));
    } finally {
      this.busy = false;
      this.save.disabled = false;
      this.skip.disabled = false;
      this.input.disabled = false;
    }
  }

  private showModelStep(): void {
    this.title.textContent = "Выберите первую модель";
    this.intro.textContent = "Выберите одну модель для папки WebUI. Этот шаг можно пропустить и скачать модель позже.";
    this.keyStep.hidden = true;
    this.modelStep.hidden = false;
    this.dialog.classList.add("first-run-dialog-models");
    this.modelSkip.focus();
    if (!this.modelsLoaded) {
      this.modelsLoaded = true;
      for (const key of ["indigo", "wai", "realistic"] as const) void this.loadModel(key);
    }
  }

  private async loadModel(key: StarterKey): Promise<void> {
    const card = this.card(key);
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    const button = card.querySelector<HTMLButtonElement>(".first-run-download");
    if (status) status.textContent = "Загружаем сведения о модели…";
    if (button) button.disabled = true;
    try {
      const model = await this.api.getStarterModel(key);
      this.models.set(key, model);
      if (status) status.textContent = model.file.name;
      if (button) { button.disabled = this.activeTaskId !== null; button.textContent = "Скачать"; }
      if (model.previewUrl) {
        const image = card.querySelector<HTMLImageElement>(".first-run-model-preview img");
        const fallback = card.querySelector<HTMLElement>(".first-run-model-preview span");
        void this.api.getPreview(model.previewUrl).then((src) => {
          if (image) { image.src = src; image.hidden = false; }
          if (fallback) fallback.hidden = true;
        }).catch(() => { /* A preview failure does not block the download. */ });
      }
    } catch (error) {
      if (status) status.textContent = `Не удалось получить файл: ${errorMessage(error)}`;
      if (button) { button.disabled = false; button.textContent = "Повторить"; }
    }
  }

  private enableModelButtons(enabled: boolean): void {
    for (const card of this.cards()) {
      const key = card.dataset.starter as StarterKey;
      const button = card.querySelector<HTMLButtonElement>(".first-run-download");
      if (button) button.disabled = !enabled || !this.models.has(key);
    }
  }

  private async selectModel(key: StarterKey): Promise<void> {
    if (this.activeTaskId || this.busy) return;
    const model = this.models.get(key);
    if (!model) return;
    const card = this.card(key);
    const name = card.querySelector("h3")?.textContent ?? model.file.name;
    const taskId = this.enqueue(model.file, "Checkpoint", name);
    if (!taskId) {
      this.modelError.textContent = "Эта модель уже есть в списке загрузок. Откройте каталог после завершения мастера.";
      return;
    }
    this.activeTaskId = taskId;
    this.activeKey = key;
    this.modelError.textContent = "Загрузка идёт в папку sd-webui/models/Stable-diffusion. Можно начать работу, пока она продолжается.";
    const progress = card.querySelector<HTMLElement>(".first-run-model-progress");
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    if (progress) progress.hidden = false;
    if (status) status.textContent = "Ожидает начала загрузки…";
    this.enableModelButtons(false);
  }

  private async finish(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.modelSkip.disabled = true;
    this.modelError.textContent = "Завершаем настройку…";
    try {
      await this.api.finishOnboarding();
      this.dialog.classList.add("first-run-closing");
      window.setTimeout(() => {
        this.dialog.close();
        this.app.inert = false;
        this.onComplete();
      }, 180);
    } catch (error) {
      this.modelError.textContent = errorMessage(error);
      this.busy = false;
      this.modelSkip.disabled = false;
    }
  }
}
