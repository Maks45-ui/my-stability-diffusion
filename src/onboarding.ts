import { startSplashScreen } from "./splash";
import { onboardingCopy, starterCopy, starterKeys, type StarterKey } from "./onboarding-copy";
import type { FlowAPI, ModelFile, StarterModel, TaskEvent } from "./types";

function required<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Элемент ${id} не найден.`);
  return node as T;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Не удалось выполнить действие.";
}

/** The model step becomes available only after WebUI has been installed. */
export class FirstRunController {
  private readonly dialog = required<HTMLDialogElement>("first-run-dialog");
  private readonly title = required<HTMLElement>("first-run-title");
  private readonly intro = required<HTMLElement>("first-run-intro");
  private readonly keyStep = required<HTMLElement>("first-run-key-step");
  private readonly modelStep = required<HTMLElement>("first-run-model-step");
  private readonly modelGrid = required<HTMLElement>("first-run-model-grid");
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
    private readonly onComplete: () => void,
    private readonly onAwaitInstallation: () => void
  ) {
    this.title.textContent = onboardingCopy.keyTitle;
    this.intro.textContent = onboardingCopy.keyIntro;
    this.modelSkip.textContent = onboardingCopy.skipModels;
    this.modelGrid.replaceChildren(...starterKeys.map((key) => this.createCard(key)));
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
    starterKeys.forEach((key) => {
      const card = this.card(key);
      card.querySelector<HTMLButtonElement>(".first-run-download")?.addEventListener("click", () => {
        if (this.models.has(key)) void this.selectModel(key);
        else void this.loadModel(key);
      });
    });
  }

  private createCard(key: StarterKey): HTMLElement {
    const copy = starterCopy[key];
    const card = document.createElement("article");
    card.className = "first-run-model-card";
    card.dataset.starter = key;
    // Only the stable structure is HTML; all visible copy comes from the UTF-8 module.
    card.innerHTML = `<div class="first-run-model-preview"><img hidden /><span aria-hidden="true">◈</span></div><div class="first-run-model-body"><span class="eyebrow"></span><h3></h3><p class="first-run-model-version"></p><div class="first-run-model-progress" hidden><div class="first-run-model-track"><div></div></div><span class="first-run-model-percent">0%</span></div><p class="first-run-model-status" role="status"></p><button class="primary-button first-run-download" type="button" disabled></button></div>`;
    const image = card.querySelector<HTMLImageElement>("img");
    if (image) image.alt = copy.previewAlt;
    const type = card.querySelector<HTMLElement>(".eyebrow");
    if (type) type.textContent = copy.type;
    const title = card.querySelector<HTMLElement>("h3");
    if (title) title.textContent = copy.name;
    const version = card.querySelector<HTMLElement>(".first-run-model-version");
    if (version) version.textContent = copy.version;
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    if (status) status.textContent = onboardingCopy.loading;
    const button = card.querySelector<HTMLButtonElement>(".first-run-download");
    if (button) button.textContent = onboardingCopy.download;
    return card;
  }

  async start(): Promise<void> {
    this.app.inert = true;
    // Begin the disk read during the welcome animation and handle rejection immediately.
    const status = this.api.getOnboardingStage()
      .then((stage) => ({ stage, error: "" }))
      .catch((error: unknown) => ({ stage: 0, error: errorMessage(error) }));
    await startSplashScreen();
    const result = await status;
    if (result.stage >= 3 || result.stage === 1) { this.app.inert = false; return; }
    this.dialog.showModal();
    if (result.stage === 2) this.showModelStep();
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
      status.textContent = `${onboardingCopy.downloading}${speed}`;
    }
    if (event.phase === "done") {
      if (status) status.textContent = onboardingCopy.downloaded;
      this.activeTaskId = null;
      void this.finish();
    } else if (["error", "cancelled", "paused"].includes(event.phase)) {
      if (status) status.textContent = event.phase === "paused" ? onboardingCopy.paused : event.message;
      this.activeTaskId = null;
      this.modelError.textContent = onboardingCopy.chooseAnother;
      this.enableModelButtons(true);
    }
  }

  async onInstallComplete(): Promise<void> {
    // The installer writes its completion marker before emitting the done event.
    // Recheck the persisted stage so interrupted installs never unlock models.
    const stage = await this.api.getOnboardingStage();
    if (stage !== 2 || this.dialog.open) return;
    this.app.inert = true;
    this.dialog.showModal();
    this.showModelStep();
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
      if (await this.api.getOnboardingStage() === 2) this.showModelStep();
      else {
        this.dialog.close();
        this.app.inert = false;
        this.onAwaitInstallation();
      }
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
    this.title.textContent = onboardingCopy.modelTitle;
    this.intro.textContent = onboardingCopy.modelIntro;
    this.keyStep.hidden = true;
    this.modelStep.hidden = false;
    this.dialog.classList.add("first-run-dialog-models");
    this.modelSkip.focus();
    if (!this.modelsLoaded) {
      this.modelsLoaded = true;
      for (const key of starterKeys) void this.loadModel(key);
    }
  }

  private async loadModel(key: StarterKey): Promise<void> {
    const card = this.card(key);
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    const button = card.querySelector<HTMLButtonElement>(".first-run-download");
    if (status) status.textContent = onboardingCopy.loading;
    if (button) button.disabled = true;
    try {
      const model = await this.api.getStarterModel(key);
      this.models.set(key, model);
      if (status) status.textContent = model.file.name;
      if (button) { button.disabled = this.activeTaskId !== null; button.textContent = onboardingCopy.download; }
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
      if (button) { button.disabled = false; button.textContent = onboardingCopy.retry; }
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
      this.modelError.textContent = onboardingCopy.alreadyQueued;
      return;
    }
    this.activeTaskId = taskId;
    this.activeKey = key;
    this.modelError.textContent = onboardingCopy.downloadLocation;
    const progress = card.querySelector<HTMLElement>(".first-run-model-progress");
    const status = card.querySelector<HTMLElement>(".first-run-model-status");
    if (progress) progress.hidden = false;
    if (status) status.textContent = onboardingCopy.queued;
    this.enableModelButtons(false);
  }

  private async finish(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.modelSkip.disabled = true;
    this.modelError.textContent = onboardingCopy.finishing;
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
