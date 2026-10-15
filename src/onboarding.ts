import { startSplashScreen } from "./splash";
import type { FlowAPI } from "./types";

function required<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Элемент ${id} не найден.`);
  return node as T;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Не удалось сохранить настройки.";
}

/** Holds the main window until the splash and the one-time setup have finished. */
export class FirstRunController {
  private readonly dialog = required<HTMLDialogElement>("first-run-dialog");
  private readonly form = required<HTMLFormElement>("first-run-form");
  private readonly input = required<HTMLInputElement>("first-run-token");
  private readonly status = required<HTMLElement>("first-run-status");
  private readonly save = required<HTMLButtonElement>("first-run-save");
  private readonly skip = required<HTMLButtonElement>("first-run-skip");
  private readonly site = required<HTMLAnchorElement>("first-run-site");
  private busy = false;

  constructor(
    private readonly api: FlowAPI,
    private readonly app: HTMLElement,
    private readonly onComplete: () => void
  ) {
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.finish(this.input.value.trim());
    });
    this.skip.addEventListener("click", () => void this.finish(null));
    this.dialog.addEventListener("cancel", (event) => event.preventDefault());
    this.site.addEventListener("click", (event) => {
      event.preventDefault();
      void this.api.openCivitaiSite().catch((error: unknown) => this.showError(errorMessage(error)));
    });
  }

  async start(): Promise<void> {
    this.app.inert = true;
    // Begin the disk read during the welcome animation and handle rejection immediately.
    const status = this.api.getOnboardingComplete()
      .then((complete) => ({ complete, error: "" }))
      .catch((error: unknown) => ({ complete: false, error: errorMessage(error) }));
    await startSplashScreen();
    const result = await status;
    if (result.complete) { this.app.inert = false; return; }
    this.dialog.showModal();
    if (result.error) this.showError(`Не удалось проверить первый запуск: ${result.error}`);
    this.input.focus();
  }

  private showError(message: string): void {
    this.status.textContent = message;
    this.status.classList.add("error");
  }

  private async finish(token: string | null): Promise<void> {
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
      await this.api.completeOnboarding(token);
      this.input.value = "";
      this.dialog.close();
      this.app.inert = false;
      this.onComplete();
    } catch (error) {
      this.showError(errorMessage(error));
    } finally {
      this.busy = false;
      this.save.disabled = false;
      this.skip.disabled = false;
      this.input.disabled = false;
    }
  }
}
