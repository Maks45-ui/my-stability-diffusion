import { flowApi } from "./bridge";
import { convertFileSrc } from "@tauri-apps/api/core";
import { DownloadManager } from "./download-manager";
import { startSplashScreen } from "./splash";
import { ThemeController } from "./theme";
import type { AppState, GalleryImage, ModelSummary, Page, TaskEvent } from "./types";

window.flow = flowApi;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Элемент ${id} не найден.`);
  return found as T;
}

const sidebar = element<HTMLElement>("sidebar");
const navIndicator = element<HTMLElement>("nav-indicator");
const nav = element<HTMLElement>("navigation");
const toast = element<HTMLElement>("toast");
const installDialog = element<HTMLDialogElement>("install-dialog");
const webuiSettingsDialog = element<HTMLDialogElement>("webui-settings-dialog");
const webuiArgInput = element<HTMLInputElement>("webui-arg-input");
const webuiArgList = element<HTMLElement>("webui-arg-list");
const webuiSettingsState = element<HTMLElement>("webui-settings-state");
const webuiSaveButton = element<HTMLButtonElement>("save-webui-settings");
const imageDialog = element<HTMLDialogElement>("image-dialog");
const terminal = element<HTMLElement>("terminal-log");
const consolePanel = element<HTMLElement>("console-panel");
const consoleToggle = element<HTMLButtonElement>("toggle-terminal");
const consoleCancel = element<HTMLButtonElement>("cancel-terminal-install");
const consoleStatus = element<HTMLElement>("console-status");
const packageList = element<HTMLElement>("package-list");
const galleryGrid = element<HTMLElement>("gallery-grid");
const galleryEmpty = element<HTMLElement>("gallery-empty");
const galleryStatus = element<HTMLElement>("gallery-status");
const galleryCount = element<HTMLElement>("gallery-count");
const galleryRefresh = element<HTMLButtonElement>("refresh-gallery");
const modelGrid = element<HTMLElement>("model-grid");
const catalogStatus = element<HTMLElement>("catalog-status");
const searchInput = element<HTMLInputElement>("model-search");
const downloadManager = new DownloadManager(
  flowApi,
  element<HTMLElement>("download-list"),
  element<HTMLElement>("download-count"),
  notify
);
new ThemeController(
  element<HTMLElement>("theme-standard"),
  element<HTMLElement>("theme-extra"),
  element<HTMLElement>("theme-state"),
  element<HTMLElement>("theme-toggle-label"),
  element<HTMLElement>("theme-toggle-icon")
);
let hasLoadedCatalog = false;
let searchSequence = 0;
let gallerySequence = 0;
let stateSequence = 0;
let debounceTimer: number | undefined;
let toastTimer: number | undefined;
let installing = false;
let stoppingWebui = false;
let installationDir = "";
let logFrame: number | undefined;
let terminalHasOutput = false;
let webuiDraft: string[] = [];
let webuiSettingsRequest = 0;
let webuiSettingsReady = false;
const pendingLogs = new Map<HTMLElement, string>();

function queueLog(target: HTMLElement, message: string): void {
  const line = message.replace(/\r/g, "").trimEnd();
  if (!line) return;
  pendingLogs.set(target, ((pendingLogs.get(target) ?? "") + line + "\n").slice(-100_000));
  if (logFrame === undefined) logFrame = window.requestAnimationFrame(() => {
    logFrame = undefined;
    for (const [output, pending] of pendingLogs) {
      if (output === terminal && !terminalHasOutput) { terminal.textContent = ""; terminalHasOutput = true; }
      output.textContent = ((output.textContent ?? "") + pending).slice(-100_000);
      output.scrollTop = output.scrollHeight;
    }
    pendingLogs.clear();
  });
}

function setConsoleStatus(label: string, state: "idle" | "running" | "done" | "error"): void {
  consoleStatus.textContent = label;
  consoleStatus.dataset.state = state;
  consoleToggle.dataset.state = state;
  consoleToggle.title = state === "running" ? `${label} — открыть логи` : "Открыть логи";
}

function setConsoleVisible(visible: boolean): void {
  consolePanel.classList.toggle("visible", visible);
  consolePanel.setAttribute("aria-hidden", String(!visible));
  consolePanel.inert = !visible;
  consoleToggle.setAttribute("aria-expanded", String(visible));
  consoleToggle.title = visible ? "Скрыть логи" : "Открыть логи";
}

function setInstalling(value: boolean): void {
  installing = value;
  element<HTMLButtonElement>("start-install").disabled = value;
  consoleCancel.hidden = !value;
  consoleCancel.disabled = false;
}

function showInstallationDir(path: string): void {
  installationDir = path;
  element<HTMLInputElement>("install-dir-settings").value = path;
  element<HTMLInputElement>("install-dir-dialog").value = path;
}

async function browseInstallationDir(): Promise<void> {
  try {
    const selected = await window.flow.browseInstallationDirectory(installationDir);
    if (selected === null) return;
    // Persist the chosen parent folder before the installer starts.
    await window.flow.saveInstallationDirectory(selected);
    // get_state returns the Windows path without the verbatim namespace prefix.
    await refreshState();
    await downloadManager.restore();
  } catch (error) { notify(message(error)); }
}

function message(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Произошла неизвестная ошибка.";
}

function notify(value: string): void {
  toast.textContent = value;
  toast.classList.add("visible");
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("visible"), 4500);
}

function make<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function webuiArgumentName(argument: string): string {
  return (argument.trim().split(/[=\s]/, 1)[0] ?? "").toLowerCase();
}

function setWebuiSettingsMessage(value: string, error = false): void {
  webuiSettingsState.textContent = value;
  webuiSettingsState.classList.toggle("error", error);
}

function renderWebuiArguments(): void {
  webuiArgList.replaceChildren();
  if (webuiDraft.length === 0) webuiArgList.append(make("span", "webui-args-empty", "Аргументы пока не добавлены."));
  for (const argument of webuiDraft) {
    const chip = make("span", "webui-arg-chip");
    const remove = make("button", "webui-arg-remove", "×");
    remove.type = "button";
    remove.setAttribute("aria-label", `Удалить аргумент ${argument}`);
    remove.addEventListener("click", () => {
      webuiDraft = webuiDraft.filter((item) => item !== argument);
      renderWebuiArguments();
      setWebuiSettingsMessage("Есть несохранённые изменения");
    });
    chip.append(make("code", "", argument), remove);
    webuiArgList.append(chip);
  }
  document.querySelectorAll<HTMLButtonElement>(".webui-quick-arg").forEach((button) => {
    button.setAttribute("aria-pressed", String(webuiDraft.some((item) => webuiArgumentName(item) === button.dataset.argument)));
  });
}

function addWebuiArgument(raw: string): boolean {
  if (!webuiSettingsReady) return false;
  const argument = raw.trim().replace(/\s+/g, " ");
  const [first, ...value] = argument.split(" ");
  const flag = first ?? "";
  const name = webuiArgumentName(argument);
  if (argument.length > 256 || !/^--[A-Za-z][A-Za-z0-9_-]*(?:=\S+)?$/.test(flag) || (flag.includes("=") && value.length > 0) || value.join(" ").startsWith("--")) {
    setWebuiSettingsMessage("Добавляйте по одному аргументу: --имя или --имя значение.", true);
    return false;
  }
  if (webuiDraft.length >= 32) { setWebuiSettingsMessage("Можно сохранить не более 32 аргументов.", true); return false; }
  if (webuiDraft.some((item) => webuiArgumentName(item) === name)) { setWebuiSettingsMessage("Этот аргумент уже добавлен.", true); return false; }
  webuiDraft.push(argument);
  renderWebuiArguments();
  setWebuiSettingsMessage("Есть несохранённые изменения");
  return true;
}

async function openWebuiSettings(): Promise<void> {
  if (webuiSettingsDialog.open) return;
  webuiSettingsDialog.showModal();
  webuiSettingsReady = false;
  webuiSaveButton.disabled = true;
  webuiArgInput.disabled = true;
  webuiArgInput.value = "";
  webuiDraft = [];
  renderWebuiArguments();
  setWebuiSettingsMessage("Загрузка настроек…");
  const request = ++webuiSettingsRequest;
  try {
    const saved = await window.flow.getWebUIArguments();
    if (request !== webuiSettingsRequest || !webuiSettingsDialog.open) return;
    webuiDraft = [...saved];
    webuiSettingsReady = true;
    webuiSaveButton.disabled = false;
    webuiArgInput.disabled = false;
    renderWebuiArguments();
    setWebuiSettingsMessage("Настройки загружены");
    webuiArgInput.focus();
  } catch (error) { setWebuiSettingsMessage(message(error), true); }
}

function updateIndicator(): void {
  const active = nav.querySelector<HTMLButtonElement>(".nav-item.active");
  if (!active) return;
  navIndicator.style.transform = `translateY(${active.offsetTop}px)`;
  navIndicator.style.height = `${active.offsetHeight}px`;
}

function showPage(page: Page): void {
  document.querySelectorAll<HTMLElement>(".page").forEach((section) => section.classList.toggle("active", section.id === `page-${page}`));
  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
    const active = button.dataset.page === page;
    button.classList.toggle("active", active);
    if (active) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current");
  });
  window.requestAnimationFrame(updateIndicator);
  void window.flow.setPage(page).catch(() => { /* Discord may be unavailable. */ });
  if (page === "models" && !hasLoadedCatalog) {
    hasLoadedCatalog = true;
    void searchModels();
  }
  if (page === "gallery") void refreshGallery();
}

function renderPackages(state: AppState): void {
  packageList.replaceChildren();
  const count = state.packages.length;
  element<HTMLElement>("package-count").textContent = String(count);
  element<HTMLElement>("package-badge").textContent = `${count} ${count === 1 ? "пакет" : "пакетов"}`;
  element<HTMLElement>("models-path").textContent = state.modelsPath;
  element<HTMLElement>("token-state").textContent = state.hasCivitaiToken ? "Сохранён" : "Не задан";
  showInstallationDir(state.installationDir);
  if (count === 0) {
    const empty = make("div", "empty-package");
    empty.append(make("div", "empty-package-icon", "◈"), make("h3", "", "Библиотека пока пуста"), make("p", "", "Добавьте первый пакет, чтобы начать работу."));
    packageList.append(empty);
    return;
  }
  for (const item of state.packages) {
    const card = make("article", "package-card glass-surface");
    const icon = make("div", "package-icon", "A");
    const details = make("div", "package-details");
    details.append(make("h3", "", item.name), make("p", "", `${item.version} · Stable Diffusion`), make("small", "", item.path));
    const badge = make("span", "package-state", item.stopping ? "● Останавливается" : item.running ? "● Запущен" : item.stopped ? "● Остановлен" : "● Готов");
    badge.dataset.state = item.stopping ? "stopping" : item.running ? "running" : item.stopped ? "stopped" : "ready";
    card.append(icon, details, badge);
    const actions = make("div", "package-actions");
    const launch = make("button", "primary-button", "Запустить");
    launch.type = "button";
    launch.disabled = item.running || stoppingWebui;
    launch.addEventListener("click", async () => {
      launch.disabled = true;
      setConsoleVisible(true);
      setConsoleStatus("Запуск WebUI", "running");
      queueLog(terminal, "[WebUI] Запуск...");
      try {
        await window.flow.launchWebUI();
        await refreshState();
      }
      catch (error) {
        setConsoleStatus("Ошибка WebUI", "error");
        queueLog(terminal, `[WebUI] ${message(error)}`);
        notify(message(error));
        await refreshState();
      }
    });
    const stop = make("button", "secondary-button package-stop", "Остановить");
    stop.type = "button";
    stop.disabled = !item.running || item.stopping || stoppingWebui;
    stop.addEventListener("click", async () => {
      if (stoppingWebui) return;
      stoppingWebui = true;
      stop.disabled = true;
      launch.disabled = true;
      setConsoleVisible(true);
      setConsoleStatus("Остановка WebUI", "running");
      try {
        await window.flow.stopWebUI();
        await refreshState();
      } catch (error) {
        stoppingWebui = false;
        queueLog(terminal, `[WebUI] ${message(error)}`);
        notify(message(error));
        await refreshState();
      }
    });
    const settings = make("button", "secondary-button", "Настройки");
    settings.type = "button";
    settings.prepend(make("span", "package-settings-icon", "⚙"));
    settings.addEventListener("click", () => void openWebuiSettings());
    actions.append(launch, stop, settings);
    card.append(actions);
    packageList.append(card);
  }
}

async function refreshState(): Promise<void> {
  const sequence = ++stateSequence;
  try {
    const state = await window.flow.getState();
    if (sequence === stateSequence) renderPackages(state);
  } catch (error) {
    if (sequence === stateSequence) notify(message(error));
  }
}

const imageObserver = new IntersectionObserver((entries, observer) => {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue;
    observer.unobserve(entry.target);
    const image = entry.target;
    if (!(image instanceof HTMLImageElement)) continue;
    const url = image.dataset.url;
    if (!url) continue;
    void window.flow.getPreview(url).then((source) => { image.src = source; image.classList.add("loaded"); }).catch(() => {
      image.alt = "Превью недоступно";
      image.parentElement?.classList.add("image-error");
    });
  }
}, { rootMargin: "180px" });

async function showImage(url: string): Promise<void> {
  const image = element<HTMLImageElement>("full-image");
  image.onerror = null;
  image.alt = "Пример изображения модели";
  image.removeAttribute("src");
  imageDialog.showModal();
  try { image.src = await window.flow.getPreview(url); }
  catch (error) { imageDialog.close(); notify(message(error)); }
}

function showGalleryImage(item: GalleryImage): void {
  const image = element<HTMLImageElement>("full-image");
  image.removeAttribute("src");
  image.alt = item.fileName;
  image.onerror = () => {
    imageDialog.close();
    notify("Не удалось открыть изображение. Обновите галерею и попробуйте снова.");
  };
  imageDialog.showModal();
  image.src = convertFileSrc(item.path);
}

function renderGalleryImage(item: GalleryImage): HTMLElement {
  const button = make("button", "gallery-card glass-surface");
  button.type = "button";
  button.title = item.fileName;
  button.setAttribute("aria-label", `Открыть изображение ${item.fileName}`);
  const image = make("img", "gallery-thumbnail");
  image.alt = item.fileName;
  image.loading = "lazy";
  image.decoding = "async";
  image.addEventListener("error", () => button.classList.add("image-error"));
  image.src = convertFileSrc(item.path);
  const caption = make("span", "gallery-caption");
  caption.append(
    make("span", "gallery-kind", item.kind === "txt2img" ? "txt2img" : "img2img"),
    make("span", "gallery-file-name", item.fileName),
    make("span", "gallery-date", item.modifiedAt ? new Date(item.modifiedAt).toLocaleDateString("ru-RU") : "")
  );
  button.append(image, caption);
  button.addEventListener("click", () => showGalleryImage(item));
  return button;
}

async function refreshGallery(): Promise<void> {
  const sequence = ++gallerySequence;
  galleryRefresh.disabled = true;
  galleryStatus.hidden = false;
  galleryStatus.textContent = "Ищем изображения WebUI…";
  galleryEmpty.hidden = true;
  galleryGrid.replaceChildren();
  try {
    const images = await window.flow.listGalleryImages();
    if (sequence !== gallerySequence) return;
    galleryCount.textContent = `${images.length} изображений`;
    galleryStatus.hidden = true;
    galleryEmpty.hidden = images.length > 0;
    const fragment = document.createDocumentFragment();
    for (const image of images) fragment.append(renderGalleryImage(image));
    galleryGrid.append(fragment);
  } catch (error) {
    if (sequence !== gallerySequence) return;
    galleryStatus.textContent = `Не удалось загрузить галерею: ${message(error)}`;
  } finally {
    if (sequence === gallerySequence) galleryRefresh.disabled = false;
  }
}

function renderModel(model: ModelSummary): HTMLElement {
  const card = make("article", "model-card glass-surface");
  const gallery = make("div", "model-gallery");
  for (const url of model.images) {
    const button = make("button", "model-image");
    button.type = "button";
    button.title = "Открыть изображение";
    const image = make("img");
    image.alt = `Пример: ${model.name}`;
    image.loading = "lazy";
    image.dataset.url = url;
    imageObserver.observe(image);
    button.append(image);
    button.addEventListener("click", () => void showImage(url));
    gallery.append(button);
  }
  if (!model.images.length) gallery.append(make("div", "no-preview", "Нет изображений"));
  const body = make("div", "model-body");
  const type = make("span", "model-type", model.type);
  const title = make("h2", "", model.name);
  const id = make("span", "model-id", `ID ${model.id}`);
  const head = make("div", "model-head");
  head.append(title, id);
  body.append(type, head);
  const footer = make("div", "model-footer");
  if (model.files.length) {
    const select = make("select", "file-select");
    select.setAttribute("aria-label", `Файл модели ${model.name}`);
    for (const [index, file] of model.files.entries()) {
      const option = make("option", "", file.name);
      option.value = String(index);
      select.append(option);
    }
    const button = make("button", "download-button", "Скачать ↓");
    button.type = "button";
    button.addEventListener("click", () => {
      const file = model.files[Number(select.value)];
      if (!file) return;
      downloadManager.enqueue(file, model.type, model.name);
    });
    footer.append(select, button);
  } else footer.append(make("span", "muted", "Файлы недоступны"));
  body.append(footer);
  card.append(gallery, body);
  return card;
}

async function searchModels(): Promise<void> {
  const sequence = ++searchSequence;
  catalogStatus.textContent = "Загружаем модели…";
  catalogStatus.hidden = false;
  modelGrid.replaceChildren();
  try {
    const models = await window.flow.searchModels(searchInput.value);
    if (sequence !== searchSequence) return;
    catalogStatus.hidden = models.length > 0;
    if (!models.length) catalogStatus.textContent = "Модели не найдены. Попробуйте другой запрос.";
    modelGrid.append(...models.map(renderModel));
  } catch (error) {
    if (sequence !== searchSequence) return;
    catalogStatus.textContent = message(error);
  }
}

function onTask(event: TaskEvent): void {
  if (event.kind === "install" || event.kind === "launch") {
    queueLog(terminal, `[${event.kind === "install" ? "Установка" : "WebUI"}] ${event.message}`);
  }
  if (event.kind === "install") {
    if (["checking", "cloning", "venv", "dependencies"].includes(event.phase)) setConsoleStatus("Установка", "running");
    if (["done", "error", "cancelled"].includes(event.phase)) {
      setConsoleStatus(event.phase === "done" ? "Установлено" : event.phase === "cancelled" ? "Отменено" : "Ошибка установки", event.phase === "done" ? "done" : "error");
      setInstalling(false);
      if (event.phase === "done") { notify("WebUI установлен."); void refreshState(); }
      else notify(event.message);
    }
    return;
  }
  if (event.kind === "launch") {
    if (event.phase === "running") setConsoleStatus("WebUI работает", "running");
    if (event.phase === "stopping") setConsoleStatus("Остановка WebUI", "running");
    if (event.phase === "done" || event.phase === "error") {
      stoppingWebui = false;
      setConsoleStatus(event.phase === "done" ? "WebUI остановлен" : "Ошибка WebUI", event.phase === "done" ? "done" : "error");
      if (event.phase === "error") notify(event.message);
      void refreshState();
    }
    return;
  }
  downloadManager.onTask(event);
}

document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
  button.addEventListener("click", () => {
    const page = button.dataset.page;
    if (page === "home" || page === "gallery" || page === "models" || page === "settings") showPage(page);
  });
});
const menuToggle = element<HTMLButtonElement>("menu-toggle");
let sidebarAnimating = false;
let sidebarAnimationTimer: number | undefined;

function finishSidebarAnimation(): void {
  if (!sidebarAnimating) return;
  sidebarAnimating = false;
  window.clearTimeout(sidebarAnimationTimer);
  sidebar.classList.remove("is-animating");
  window.requestAnimationFrame(updateIndicator);
}

sidebar.addEventListener("transitionend", (event: TransitionEvent) => {
  if (event.target === sidebar && event.propertyName === "width") finishSidebarAnimation();
});
sidebar.addEventListener("transitioncancel", (event: TransitionEvent) => {
  if (event.target === sidebar && event.propertyName === "width") finishSidebarAnimation();
});

menuToggle.addEventListener("click", () => {
  if (sidebarAnimating) return;
  sidebarAnimating = true;
  sidebar.classList.add("is-animating");
  // Apply hidden labels before changing width so text cannot overlap during motion.
  void sidebar.offsetWidth;
  const collapsed = !sidebar.classList.contains("collapsed");
  sidebar.classList.toggle("collapsed", collapsed);
  const label = collapsed ? "Развернуть меню" : "Свернуть меню";
  menuToggle.setAttribute("aria-label", label);
  menuToggle.setAttribute("title", label);
  menuToggle.setAttribute("aria-expanded", String(!collapsed));
  // Covers reduced motion and cases where a width transition is cancelled.
  sidebarAnimationTimer = window.setTimeout(finishSidebarAnimation, 360);
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || window.matchMedia("(max-width: 760px)").matches) {
    finishSidebarAnimation();
  }
});
element<HTMLButtonElement>("add-package").addEventListener("click", () => {
  if (installing) setConsoleVisible(true);
  else installDialog.showModal();
});
element<HTMLButtonElement>("close-install").addEventListener("click", () => installDialog.close());
element<HTMLButtonElement>("start-install").addEventListener("click", async () => {
  if (installing) return;
  if (!installationDir) { notify("Сначала выберите папку установки."); return; }
  setInstalling(true);
  installDialog.close();
  setConsoleVisible(true);
  setConsoleStatus("Установка", "running");
  queueLog(terminal, "[Установка] Начало установки AUTOMATIC1111 WebUI...");
  try { await window.flow.installWebUI(installationDir); }
  catch (error) {
    setInstalling(false);
    setConsoleStatus("Ошибка установки", "error");
    queueLog(terminal, `[Установка] ${message(error)}`);
    notify(message(error));
  }
});
consoleToggle.addEventListener("click", () => setConsoleVisible(!consolePanel.classList.contains("visible")));
element<HTMLButtonElement>("close-terminal").addEventListener("click", () => {
  setConsoleVisible(false);
  consoleToggle.focus();
});
element<HTMLButtonElement>("clear-terminal").addEventListener("click", () => {
  pendingLogs.delete(terminal);
  terminalHasOutput = true;
  terminal.textContent = "";
});
element<HTMLButtonElement>("cancel-install").addEventListener("click", () => installDialog.close());
element<HTMLFormElement>("webui-arg-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if (addWebuiArgument(webuiArgInput.value)) webuiArgInput.value = "";
});
document.querySelectorAll<HTMLButtonElement>(".webui-quick-arg").forEach((button) => {
  button.addEventListener("click", () => {
    const argument = button.dataset.argument;
    if (!argument || !webuiSettingsReady) return;
    if (webuiDraft.some((item) => webuiArgumentName(item) === argument)) {
      webuiDraft = webuiDraft.filter((item) => webuiArgumentName(item) !== argument);
      renderWebuiArguments();
      setWebuiSettingsMessage("Есть несохранённые изменения");
    } else addWebuiArgument(argument);
  });
});
element<HTMLButtonElement>("close-webui-settings").addEventListener("click", () => webuiSettingsDialog.close());
element<HTMLButtonElement>("cancel-webui-settings").addEventListener("click", () => webuiSettingsDialog.close());
webuiSettingsDialog.addEventListener("close", () => { ++webuiSettingsRequest; webuiSettingsReady = false; });
webuiSaveButton.addEventListener("click", async () => {
  if (!webuiSettingsReady) return;
  const request = webuiSettingsRequest;
  const argumentsToSave = [...webuiDraft];
  webuiSaveButton.disabled = true;
  try {
    await window.flow.saveWebUIArguments(argumentsToSave);
    if (request === webuiSettingsRequest && webuiSettingsDialog.open) webuiSettingsDialog.close();
    notify("Аргументы WebUI сохранены. Они применятся при следующем запуске.");
  } catch (error) {
    if (request === webuiSettingsRequest && webuiSettingsDialog.open) {
      setWebuiSettingsMessage(message(error), true);
      webuiSaveButton.disabled = false;
    }
  }
});
consoleCancel.addEventListener("click", async () => {
  if (!installing) return;
  consoleCancel.disabled = true;
  try { await window.flow.cancelInstall(); }
  catch (error) { consoleCancel.disabled = false; notify(message(error)); }
});
element<HTMLButtonElement>("close-image").addEventListener("click", () => imageDialog.close());
galleryRefresh.addEventListener("click", () => void refreshGallery());
element<HTMLButtonElement>("search-button").addEventListener("click", () => void searchModels());
searchInput.addEventListener("input", () => {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(() => void searchModels(), 400);
});
searchInput.addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); void searchModels(); } });
element<HTMLButtonElement>("browse-install-settings").addEventListener("click", () => void browseInstallationDir());
element<HTMLButtonElement>("browse-install-dialog").addEventListener("click", () => void browseInstallationDir());
element<HTMLButtonElement>("theme-toggle").addEventListener("click", () => {
  showPage("settings");
  element<HTMLElement>("theme-picker").scrollIntoView({ behavior: "smooth", block: "start" });
});
element<HTMLFormElement>("token-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = element<HTMLInputElement>("token-input");
  try { await window.flow.saveCivitaiToken(input.value); input.value = ""; notify("Ключ Civitai сохранён."); await refreshState(); }
  catch (error) { notify(message(error)); }
});
window.addEventListener("resize", updateIndicator);
void window.flow.onTask(onTask).catch((error: unknown) => notify(message(error)));
startSplashScreen();
void refreshState();
void downloadManager.restore().catch((error: unknown) => notify(message(error)));
void window.flow.setPage("home").catch(() => { /* Discord may be unavailable. */ });
window.requestAnimationFrame(updateIndicator);
