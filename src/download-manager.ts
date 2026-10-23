import type { DownloadedFile, FlowAPI, ModelFile, TaskEvent } from "./types";

type DownloadStatus = "queued" | "downloading" | "pausing" | "paused" | "cancelling" | "completed" | "error";

interface DownloadItem {
  id: string;
  modelName: string;
  modelType: string;
  file: ModelFile;
  status: DownloadStatus;
  percent: number | null;
  bytesDownloaded: number;
  totalBytes: number;
  bytesPerSecond: number;
  error: string | null;
  resume: boolean;
}

const STATUS_TEXT: Record<DownloadStatus, string> = {
  queued: "В очереди",
  downloading: "Скачивается",
  pausing: "Приостанавливается",
  paused: "Пауза",
  cancelling: "Отменяется",
  completed: "В библиотеке",
  error: "Ошибка"
};

function node<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}

function formatBytes(value: number): string {
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(0)} КБ`;
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} МБ`;
  return `${(value / 1024 / 1024 / 1024).toFixed(2)} ГБ`;
}

function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "Осталось: —";
  const rounded = Math.ceil(seconds);
  if (rounded < 60) return `Осталось: ~${rounded} с`;
  if (rounded < 3600) return `Осталось: ~${Math.ceil(rounded / 60)} мин`;
  return `Осталось: ~${Math.floor(rounded / 3600)} ч ${Math.ceil((rounded % 3600) / 60)} мин`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === "string" ? error : "Не удалось выполнить действие.";
}

/** Serializes downloads because the Rust worker accepts one active transfer at a time. */
export class DownloadManager {
  private readonly items = new Map<string, DownloadItem>();
  private readonly cards = new Map<string, HTMLElement>();
  private activeId: string | null = null;

  constructor(
    private readonly api: FlowAPI,
    private readonly list: HTMLElement,
    private readonly counter: HTMLElement,
    private readonly notify: (message: string) => void
  ) {
    this.renderEmpty();
  }

  async restore(): Promise<void> {
    const files = await this.api.listDownloads();
    this.items.clear();
    this.cards.clear();
    this.activeId = null;
    this.list.replaceChildren();
    for (const file of files) this.addCompleted(file);
    this.renderEmpty();
    this.updateCount();
  }

  enqueue(file: ModelFile, modelType: string, modelName: string): string | null {
    const duplicate = [...this.items.values()].some((item) => item.file.name === file.name);
    if (duplicate) { this.notify("Этот файл уже находится в списке загрузок."); return null; }
    const item: DownloadItem = {
      id: crypto.randomUUID(), modelName, modelType, file, status: "queued",
      percent: 0, bytesDownloaded: 0, totalBytes: Math.max(0, file.sizeKb * 1024),
      bytesPerSecond: 0, error: null, resume: false
    };
    this.add(item);
    this.pump();
    return item.id;
  }

  onTask(event: TaskEvent): void {
    if (event.kind !== "download" || !event.taskId) return;
    const item = this.items.get(event.taskId);
    if (!item) return;
    if (event.bytesDownloaded !== undefined) item.bytesDownloaded = event.bytesDownloaded;
    if (event.totalBytes !== undefined) item.totalBytes = event.totalBytes;
    if (event.bytesPerSecond !== undefined) item.bytesPerSecond = event.bytesPerSecond;
    if (event.percent !== undefined) item.percent = event.percent;

    if (event.phase === "done") {
      item.status = "completed";
      item.percent = 100;
      item.error = null;
      this.notify(`${item.file.name} загружен.`);
    } else if (event.phase === "paused") {
      item.status = "paused";
      item.resume = true;
      item.bytesPerSecond = 0;
    } else if (event.phase === "cancelled") {
      this.remove(item.id);
    } else if (event.phase === "error") {
      item.status = "error";
      item.error = event.message;
      this.notify(event.message);
    }

    if (event.phase !== "cancelled") this.update(item);
    if (["done", "paused", "cancelled", "error"].includes(event.phase)) {
      if (this.activeId === item.id) this.activeId = null;
      this.pump();
    }
  }

  private addCompleted(file: DownloadedFile): void {
    const duplicate = [...this.items.values()].some((item) => item.file.name === file.name);
    if (duplicate) return;
    const paused = file.status === "paused";
    this.add({
      id: crypto.randomUUID(), modelName: file.name, modelType: file.modelType,
      file: { name: file.name, downloadUrl: file.downloadUrl ?? "", sizeKb: file.sizeBytes / 1024 },
      status: paused ? "paused" : "completed",
      percent: paused ? (file.sizeBytes > 0 ? Math.min(100, Math.floor(file.downloadedBytes / file.sizeBytes * 100)) : null) : 100,
      bytesDownloaded: file.downloadedBytes,
      totalBytes: file.sizeBytes, bytesPerSecond: 0, error: null, resume: paused
    });
  }

  private add(item: DownloadItem): void {
    this.items.set(item.id, item);
    const card = this.makeCard(item);
    this.cards.set(item.id, card);
    if (this.list.querySelector(".downloads-empty")) this.list.replaceChildren();
    this.list.prepend(card);
    this.update(item);
    this.updateCount();
  }

  private remove(id: string): void {
    this.items.delete(id);
    this.cards.get(id)?.remove();
    this.cards.delete(id);
    this.renderEmpty();
    this.updateCount();
  }

  private renderEmpty(): void {
    if (this.items.size > 0) return;
    this.list.replaceChildren(node("div", "downloads-empty", "Выбранные модели и ход загрузки появятся здесь."));
  }

  private updateCount(): void {
    const active = [...this.items.values()].filter((item) => ["queued", "downloading", "pausing", "cancelling"].includes(item.status)).length;
    this.counter.textContent = active > 0 ? `${active} в работе · ${this.items.size} всего` : `${this.items.size} файлов`;
  }

  private makeCard(item: DownloadItem): HTMLElement {
    const card = node("article", "download-card glass-surface");
    card.dataset.id = item.id;
    const icon = node("span", "download-card-icon", item.modelType.toLowerCase().includes("lora") ? "✦" : "◈");
    icon.setAttribute("aria-hidden", "true");
    const content = node("div", "download-card-content");
    const heading = node("div", "download-card-heading");
    const title = node("div", "download-card-title");
    title.append(node("span", "download-card-type", item.modelType), node("h3", "", item.modelName), node("small", "", item.file.name));
    heading.append(title, node("span", "download-badge"));
    const metrics = node("div", "download-metrics");
    metrics.append(node("strong", "download-percent"), node("span", "download-size"), node("span", "download-speed"), node("span", "download-eta"));
    const track = node("div", "download-track");
    track.setAttribute("role", "progressbar");
    track.setAttribute("aria-label", `Загрузка ${item.file.name}`);
    track.setAttribute("aria-valuemin", "0");
    track.setAttribute("aria-valuemax", "100");
    track.append(node("div", "download-fill"));
    content.append(heading, metrics, track, node("p", "download-error"), node("div", "download-actions"));
    card.append(icon, content);
    return card;
  }

  private action(label: string, handler: () => void, variant = "quiet"): HTMLButtonElement {
    const button = node("button", `download-action download-action-${variant}`, label);
    button.type = "button";
    button.addEventListener("click", handler);
    return button;
  }

  private update(item: DownloadItem): void {
    const card = this.cards.get(item.id);
    if (!card) return;
    const statusChanged = card.dataset.status !== item.status;
    card.dataset.status = item.status;
    if (statusChanged) {
      const badge = node("span", "download-badge", STATUS_TEXT[item.status]);
      card.querySelector(".download-badge")?.replaceWith(badge);
      const actions = card.querySelector<HTMLElement>(".download-actions");
      if (actions) {
        const buttons: HTMLButtonElement[] = [];
        if (item.status === "queued") buttons.push(this.action("Отмена", () => this.remove(item.id)));
        if (item.status === "downloading") {
          buttons.push(this.action("Пауза", () => void this.pause(item), "primary"));
          buttons.push(this.action("Отмена", () => void this.cancel(item)));
        }
        if (item.status === "paused") {
          if (item.file.downloadUrl) buttons.push(this.action("Продолжить", () => this.resume(item), "primary"));
          buttons.push(this.action("Удалить", () => void this.delete(item)));
        }
        if (item.status === "completed") buttons.push(this.action("Удалить", () => void this.delete(item)));
        if (item.status === "error") {
          buttons.push(this.action("Повторить", () => void this.retry(item), "primary"));
          buttons.push(this.action("Удалить", () => void this.delete(item)));
        }
        actions.replaceChildren(...buttons);
      }
      this.updateCount();
    }
    const percent = item.percent;
    const percentNode = card.querySelector<HTMLElement>(".download-percent");
    if (percentNode) percentNode.textContent = percent === null ? "—" : `${percent}%`;
    const size = card.querySelector<HTMLElement>(".download-size");
    if (size) size.textContent = item.totalBytes > 0 ? `${formatBytes(item.bytesDownloaded)} / ${formatBytes(item.totalBytes)}` : formatBytes(item.bytesDownloaded);
    const speed = card.querySelector<HTMLElement>(".download-speed");
    if (speed) speed.textContent = item.status === "downloading" && item.bytesPerSecond > 0 ? `${formatBytes(item.bytesPerSecond)}/с` : "";
    const eta = card.querySelector<HTMLElement>(".download-eta");
    if (eta) eta.textContent = item.status === "downloading" && item.totalBytes > 0 && item.bytesPerSecond > 0
      ? formatEta((item.totalBytes - item.bytesDownloaded) / item.bytesPerSecond) : "";
    const fill = card.querySelector<HTMLElement>(".download-fill");
    if (fill) fill.style.width = `${percent ?? 0}%`;
    const track = card.querySelector<HTMLElement>(".download-track");
    if (track) {
      if (percent === null) track.removeAttribute("aria-valuenow");
      else track.setAttribute("aria-valuenow", String(percent));
    }
    const error = card.querySelector<HTMLElement>(".download-error");
    if (error) { error.textContent = item.error ?? ""; error.hidden = item.error === null; }
  }

  private pump(): void {
    if (this.activeId) return;
    const next = [...this.items.values()].find((item) => item.status === "queued");
    if (!next) return;
    this.activeId = next.id;
    next.status = "downloading";
    next.error = null;
    this.update(next);
    void this.api.downloadModel(next.file, next.modelType, next.id, next.resume).catch((error: unknown) => {
      next.status = "error";
      next.error = errorMessage(error);
      this.activeId = null;
      this.update(next);
      this.notify(next.error);
      this.pump();
    });
  }

  private async pause(item: DownloadItem): Promise<void> {
    item.status = "pausing";
    this.update(item);
    try { await this.api.pauseDownload(); }
    catch (error) { item.status = "downloading"; this.update(item); this.notify(errorMessage(error)); }
  }

  private async cancel(item: DownloadItem): Promise<void> {
    item.status = "cancelling";
    this.update(item);
    try { await this.api.cancelDownload(); }
    catch (error) { item.status = "downloading"; this.update(item); this.notify(errorMessage(error)); }
  }

  private resume(item: DownloadItem): void {
    item.status = "queued";
    this.update(item);
    this.pump();
  }

  private async retry(item: DownloadItem): Promise<void> {
    try {
      item.status = "queued";
      item.bytesPerSecond = 0;
      item.resume = true;
      item.error = null;
      this.update(item);
      this.pump();
    } catch (error) { this.notify(errorMessage(error)); }
  }

  private async delete(item: DownloadItem): Promise<void> {
    try { await this.api.deleteDownload(item.modelType, item.file.name); this.remove(item.id); }
    catch (error) { this.notify(errorMessage(error)); }
  }
}
