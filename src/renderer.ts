import { flowApi } from "./bridge";
import { convertFileSrc } from "@tauri-apps/api/core";
import { DownloadManager } from "./download-manager";
import { FirstRunController } from "./onboarding";
import { ThemeController } from "./theme";
import type { AppState, CheckReport, GalleryImage, ModelSummary, PackageInfo, Page, TaskEvent } from "./types";

window.flow = flowApi;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Элемент ${id} не найден.`);
  return found as T;
}

const sidebar = element<HTMLElement>("sidebar");
const toast = element<HTMLElement>("toast");
const installDialog = element<HTMLDialogElement>("install-dialog");
const installPackageType = element<HTMLSelectElement>("install-package-type");
const forgeDirInput = element<HTMLInputElement>("forge-dir-dialog");
const webuiSettingsDialog = element<HTMLDialogElement>("webui-settings-dialog");
const webuiArgInput = element<HTMLInputElement>("webui-arg-input");
const webuiArgList = element<HTMLElement>("webui-arg-list");
const webuiSettingsState = element<HTMLElement>("webui-settings-state");
const webuiSaveButton = element<HTMLButtonElement>("save-webui-settings");
const sdxlMemoryProfile = element<HTMLInputElement>("sdxl-memory-profile");
const xformersStatus = element<HTMLElement>("xformers-status");
const installXformersButton = element<HTMLButtonElement>("install-xformers");
const imageDialog = element<HTMLDialogElement>("image-dialog");
const terminal = element<HTMLElement>("terminal-log");
const consolePanel = element<HTMLElement>("console-panel");
const consoleToggle = element<HTMLButtonElement>("toggle-terminal");
const consoleCancel = element<HTMLButtonElement>("cancel-terminal-install");
const consoleStop = element<HTMLButtonElement>("stop-terminal-webui");
const consoleRestart = element<HTMLButtonElement>("restart-terminal-webui");
const consoleStatus = element<HTMLElement>("console-status");
const packageList = element<HTMLElement>("package-list");
const galleryGrid = element<HTMLElement>("gallery-grid");
const galleryEmpty = element<HTMLElement>("gallery-empty");
const galleryStatus = element<HTMLElement>("gallery-status");
const galleryCount = element<HTMLElement>("gallery-count");
const galleryRefresh = element<HTMLButtonElement>("refresh-gallery");
const gallerySelect = element<HTMLButtonElement>("select-gallery");
const galleryDelete = element<HTMLButtonElement>("delete-gallery");
const gallerySelectedCount = element<HTMLElement>("gallery-selected-count");
const galleryContextMenu = element<HTMLElement>("gallery-context-menu");
const galleryDeleteDialog = element<HTMLDialogElement>("gallery-delete-dialog");
const galleryDeleteConfirm = element<HTMLButtonElement>("confirm-gallery-delete");
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
  element<HTMLElement>("theme-custom"),
  element<HTMLElement>("theme-state"),
  element<HTMLElement>("theme-toggle-label"),
  element<HTMLElement>("theme-toggle-icon")
);
let hasLoadedCatalog = false;
let searchSequence = 0;
let gallerySequence = 0;
let galleryImageCount = 0;
let gallerySelectMode = false;
const selectedGalleryPaths = new Set<string>();
let contextGalleryImage: GalleryImage | null = null;
let previewGalleryImage: GalleryImage | null = null;
let stateSequence = 0;
let debounceTimer: number | undefined;
let toastTimer: number | undefined;
let installing = false;
let installingPackageId: PackageInfo["id"] = "forge-neo";
let stoppingWebui = false;
let webuiControlBusy = false;
let hasWebuiPackage = false;
let webuiRunning = false;
let installationDir = "";
let preflightSequence = 0;
let activePackageId: PackageInfo["id"] = "automatic1111";
let logFrame: number | undefined;
let terminalHasOutput = false;
let webuiDraft: string[] = [];
let webuiSettingsRequest = 0;
let webuiSettingsReady = false;
let webuiSettingsDirty = false;
const attentionFlags = new Set(["--xformers", "--opt-sub-quad-attention", "--opt-sdp-attention", "--opt-sdp-no-mem-attention"]);
const vramFlags = new Set(["--lowvram", "--medvram", "--medvram-sdxl"]);
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
  updateConsoleControls();
}

function updateConsoleControls(): void {
  const unavailable = !hasWebuiPackage || installing || stoppingWebui || webuiControlBusy;
  consoleStop.disabled = unavailable || !webuiRunning;
  consoleRestart.disabled = unavailable;
}

async function waitForWebuiStop(): Promise<void> {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline) {
    if (!(await window.flow.getWebUIStatus()).running) return;
    await new Promise<void>((resolve) => window.setTimeout(resolve, 150));
  }
  throw new Error("WebUI не завершился за 15 секунд. Повторите остановку.");
}

async function controlWebui(restart: boolean): Promise<void> {
  if (webuiControlBusy || installing || !hasWebuiPackage) return;
  webuiControlBusy = true;
  setConsoleVisible(true);
  updateConsoleControls();
  try {
    const running = (await window.flow.getWebUIStatus()).running;
    if (running) {
      stoppingWebui = true;
      updateConsoleControls();
      setConsoleStatus(restart ? "Перезапуск WebUI" : "Остановка WebUI", "running");
      queueLog(terminal, restart ? "[WebUI] Перезапуск: останавливаем процесс..." : "[WebUI] Останавливаем процесс...");
      await window.flow.stopWebUI();
      await waitForWebuiStop();
      webuiRunning = false;
      stoppingWebui = false;
    }
    if (restart) {
      queueLog(terminal, "[WebUI] Запускаем процесс заново...");
      setConsoleStatus("Запуск WebUI", "running");
      await window.flow.launchWebUI(activePackageId);
      webuiRunning = true;
    } else {
      setConsoleStatus("WebUI остановлен", "done");
    }
  } catch (error) {
    stoppingWebui = false;
    setConsoleStatus(restart ? "Ошибка перезапуска" : "Ошибка остановки", "error");
    queueLog(terminal, `[WebUI] ${message(error)}`);
    notify(message(error));
  } finally {
    webuiControlBusy = false;
    updateConsoleControls();
    await refreshState();
  }
}

function showInstallationDir(path: string): void {
  installationDir = path;
  element<HTMLInputElement>("install-dir-settings").value = path;
  element<HTMLInputElement>("install-dir-dialog").value = path;
}

function renderChecks(target: HTMLElement, report: CheckReport): void {
  target.replaceChildren();
  for (const check of report.checks) {
    const row = make("div", "check-row");
    row.dataset.level = check.level;
    row.append(make("strong", "", check.label), make("span", "", check.value));
    target.append(row);
  }
}

async function refreshPreflight(): Promise<void> {
  const request = ++preflightSequence;
  const target = element<HTMLElement>("preflight-checks");
  const importForge = installPackageType.value === "forge-neo-import";
  target.parentElement!.hidden = importForge;
  if (importForge) return;
  target.textContent = "Проверяем окружение…";
  try {
    const report = await window.flow.getPreflight("forge-neo");
    if (request !== preflightSequence || !installDialog.open) return;
    renderChecks(target, report);
    element<HTMLButtonElement>("start-install").disabled = !report.canProceed;
  } catch (error) {
    if (request === preflightSequence) target.textContent = message(error);
  }
}

async function browseInstallationDir(): Promise<void> {
  try {
    const selected = await window.flow.browseInstallationDirectory(installationDir);
    if (selected === null) return;
    // Persist the chosen parent folder before the installer starts.
    await window.flow.saveInstallationDirectory(selected);
    // get_state returns the Windows path without the verbatim namespace prefix.
    await refreshState();
    if (installDialog.open) void refreshPreflight();
    await downloadManager.restore();
  } catch (error) { notify(message(error)); }
}

async function browseModelsLibrary(): Promise<void> {
  try {
    const selected = await window.flow.browseInstallationDirectory(installationDir);
    if (!selected) return;
    await window.flow.saveModelsLibraryDirectory(selected);
    await refreshState();
    await downloadManager.restore();
    notify("Общая папка моделей сохранена. Новые загрузки пойдут туда.");
  } catch (error) { notify(message(error)); }
}

async function runPackageMaintenance(item: PackageInfo, action: "repair" | "update" | "rollback"): Promise<void> {
  if (installing) return;
  installingPackageId = item.id;
  setInstalling(true);
  setConsoleVisible(true);
  setConsoleStatus("Обслуживание пакета", "running");
  queueLog(terminal, `[Пакет] ${item.name}: ${action}`);
  try {
    if (action === "repair") await window.flow.repairPackage(item.id);
    else if (action === "update") await window.flow.updatePackage(item.id);
    else await window.flow.rollbackPackage(item.id);
  } catch (error) {
    setInstalling(false);
    setConsoleStatus("Ошибка обслуживания", "error");
    queueLog(terminal, `[Пакет] ${message(error)}`);
    notify(message(error));
  }
}

async function showPackageHealth(item: PackageInfo): Promise<void> {
  const dialog = element<HTMLDialogElement>("health-dialog");
  element<HTMLElement>("health-title").textContent = `Проверка ${item.name}`;
  element<HTMLElement>("health-checks").textContent = "Проверяем файлы, окружение и память…";
  dialog.showModal();
  try {
    const report = await window.flow.checkPackage(item.id);
    if (dialog.open) renderChecks(element<HTMLElement>("health-checks"), report);
  } catch (error) {
    if (dialog.open) element<HTMLElement>("health-checks").textContent = message(error);
  }
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
  if (value === "Есть несохранённые изменения") webuiSettingsDirty = true;
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
  if (attentionFlags.has(name)) webuiDraft = webuiDraft.filter((item) => !attentionFlags.has(webuiArgumentName(item)));
  if (vramFlags.has(name)) webuiDraft = webuiDraft.filter((item) => !vramFlags.has(webuiArgumentName(item)));
  webuiDraft.push(argument);
  renderWebuiArguments();
  setWebuiSettingsMessage("Есть несохранённые изменения");
  return true;
}

async function openWebuiSettings(): Promise<void> {
  const memoryCard = document.querySelector<HTMLElement>(".webui-memory-card");
  if (memoryCard) memoryCard.hidden = activePackageId === "forge-neo";
  document.querySelectorAll<HTMLButtonElement>(".webui-quick-arg").forEach((button) => {
    button.hidden = activePackageId === "forge-neo" && button.dataset.argument !== "--xformers"
      && button.dataset.argument !== "--api" && button.dataset.argument !== "--listen";
  });
  if (webuiSettingsDialog.open) return;
  webuiSettingsDialog.showModal();
  webuiSettingsReady = false;
  webuiSettingsDirty = false;
  webuiSaveButton.disabled = true;
  webuiArgInput.disabled = true;
  sdxlMemoryProfile.disabled = true;
  webuiArgInput.value = "";
  webuiDraft = [];
  renderWebuiArguments();
  setWebuiSettingsMessage("Загрузка настроек…");
  const request = ++webuiSettingsRequest;
  try {
    const saved = await window.flow.getWebUISettings();
    if (request !== webuiSettingsRequest || !webuiSettingsDialog.open) return;
    webuiDraft = [...saved.arguments];
    sdxlMemoryProfile.checked = saved.sdxlMemoryProfile;
    webuiSettingsReady = true;
    webuiSettingsDirty = false;
    webuiSaveButton.disabled = false;
    webuiArgInput.disabled = false;
    sdxlMemoryProfile.disabled = false;
    renderWebuiArguments();
    setWebuiSettingsMessage("Настройки загружены");
    webuiArgInput.focus();
  } catch (error) { setWebuiSettingsMessage(message(error), true); }
}

function showPage(page: Page): void {
  document.querySelectorAll<HTMLElement>(".page").forEach((section) => section.classList.toggle("active", section.id === `page-${page}`));
  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
    const active = button.dataset.page === page;
    button.classList.toggle("active", active);
    if (active) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current");
  });
  void window.flow.setPage(page).catch(() => { /* Discord may be unavailable. */ });
  if (page === "models" && !hasLoadedCatalog) {
    hasLoadedCatalog = true;
    void searchModels();
  }
  if (page === "gallery") void refreshGallery();
  else hideGalleryContextMenu();
}

function renderPackages(state: AppState): void {
  packageList.replaceChildren();
  const count = state.packages.length;
  activePackageId = state.packages.find((item) => item.active)?.id ?? "automatic1111";
  hasWebuiPackage = count > 0;
  webuiRunning = state.packages.some((item) => item.running);
  updateConsoleControls();
  element<HTMLElement>("package-count").textContent = String(count);
  element<HTMLElement>("package-badge").textContent = `${count} ${count === 1 ? "пакет" : "пакетов"}`;
  element<HTMLElement>("models-path").textContent = state.modelsPath;
  element<HTMLInputElement>("models-library-dir").value = state.modelsLibraryDir;
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
    card.dataset.active = String(item.active);
    const icon = make("div", "package-icon", item.id === "forge-neo" ? "F" : "A");
    const details = make("div", "package-details");
    details.append(make("h3", "", item.name), make("p", "", `${item.version} · Stable Diffusion`), make("small", "", item.path));
    if (item.readyUrl) details.append(make("small", "package-url", item.readyUrl));
    const badge = make("span", "package-state", item.stopping ? "Останавливается" : item.readyUrl ? "Запущен" : item.running ? "Запускается" : item.stopped ? "Остановлен" : "Готов");
    badge.dataset.state = item.stopping ? "stopping" : item.readyUrl ? "running" : item.running ? "starting" : item.stopped ? "stopped" : "ready";
    card.append(icon, details, badge);
    const actions = make("div", "package-actions");
    const launch = make("button", "primary-button", "Запустить");
    launch.type = "button";
    launch.disabled = webuiRunning || stoppingWebui || webuiControlBusy || installing;
    launch.addEventListener("click", async () => {
      launch.disabled = true;
      setConsoleVisible(true);
      setConsoleStatus("Запуск WebUI", "running");
      queueLog(terminal, "[WebUI] Запуск...");
      try {
        if (activePackageId !== item.id) await window.flow.selectPackage(item.id);
        activePackageId = item.id;
        await downloadManager.restore();
        await window.flow.launchWebUI(item.id);
        webuiRunning = true;
        updateConsoleControls();
        await refreshState();
      }
      catch (error) {
        setConsoleStatus("Ошибка WebUI", "error");
        queueLog(terminal, `[WebUI] ${message(error)}`);
        notify(message(error));
        await refreshState();
      }
    });
    const open = make("button", "secondary-button", "Открыть WebUI");
    open.type = "button";
    open.disabled = !item.readyUrl || item.stopping;
    if (item.readyUrl) open.title = item.readyUrl;
    open.addEventListener("click", async () => {
      try { await window.flow.openWebUI(); }
      catch (error) { notify(message(error)); }
    });
    const stop = make("button", "secondary-button package-stop", "Остановить");
    stop.type = "button";
    stop.disabled = !item.running || item.stopping || stoppingWebui || webuiControlBusy;
    stop.addEventListener("click", () => void controlWebui(false));
    const settings = make("button", "secondary-button", "Настройки");
    settings.type = "button";
    settings.prepend(make("span", "package-settings-icon", "⚙"));
    settings.addEventListener("click", async () => {
      try {
        if (activePackageId !== item.id) await window.flow.selectPackage(item.id);
        activePackageId = item.id;
        await downloadManager.restore();
        await refreshState();
        await openWebuiSettings();
      } catch (error) { notify(message(error)); }
    });
    actions.append(launch, open, stop, settings);
    const linkModels = make("button", "secondary-button", "Связать модели");
    linkModels.type = "button";
    linkModels.title = "Добавить существующие чекпоинты в общую библиотеку без копирования (только один диск NTFS)";
    linkModels.addEventListener("click", async () => {
      linkModels.disabled = true;
      try {
        if (activePackageId !== item.id) await window.flow.selectPackage(item.id);
        activePackageId = item.id;
        notify(await window.flow.linkPackageModels(item.id));
        await downloadManager.restore();
        await refreshState();
      } catch (error) { notify(message(error)); linkModels.disabled = false; }
    });
    actions.append(linkModels);
    if (item.managed) {
      const check = make("button", "secondary-button", "Проверить");
      check.type = "button";
      check.addEventListener("click", () => void showPackageHealth(item));
      const repair = make("button", "secondary-button", "Исправить");
      repair.type = "button";
      repair.disabled = item.running || installing;
      repair.addEventListener("click", () => void runPackageMaintenance(item, "repair"));
      const update = make("button", "secondary-button", "Обновить");
      update.type = "button";
      update.disabled = item.running || installing;
      update.addEventListener("click", () => void runPackageMaintenance(item, "update"));
      actions.append(check, repair, update);
      if (item.hasRollback) {
        const rollback = make("button", "secondary-button", "Откатить");
        rollback.type = "button";
        rollback.disabled = item.running || installing;
        rollback.addEventListener("click", () => void runPackageMaintenance(item, "rollback"));
        actions.append(rollback);
      }
    }
    card.append(actions);
    packageList.append(card);
  }
}

async function refreshState(): Promise<void> {
  const sequence = ++stateSequence;
  try {
    const state = await window.flow.getState();
    if (sequence === stateSequence) {
      renderPackages(state);
      void refreshWebUIStatus();
    }
  } catch (error) {
    if (sequence === stateSequence) notify(message(error));
  }
}

let runtimeSequence = 0;

async function refreshWebUIStatus(): Promise<void> {
  const sequence = ++runtimeSequence;
  try {
    const status = await window.flow.getWebUIStatus();
    if (sequence !== runtimeSequence) return;
    webuiRunning = status.running;
    updateConsoleControls();
    const ready = status.running && status.port !== null && status.pingMs !== null;
    const mode = ready ? "ready" : status.running ? "pending" : "offline";
    element<HTMLElement>("runtime-dot").dataset.state = mode;
    element<HTMLElement>("sidebar-status-dot").dataset.state = mode;
    element<HTMLElement>("runtime-state").textContent = ready ? "Запущен" : status.running ? "Запускается" : "Остановлен";
    element<HTMLElement>("sidebar-status-label").textContent = ready ? "WebUI запущен" : status.running ? "WebUI запускается" : "WebUI не запущен";
    element<HTMLElement>("system-status").textContent = ready ? "WebUI запущен" : status.running ? "Запуск WebUI" : "Остановлен";
    element<HTMLElement>("runtime-port").textContent = status.port === null ? "—" : `:${status.port}`;
    element<HTMLElement>("runtime-ping").textContent = status.pingMs === null ? "—" : `${status.pingMs} ms`;
    const used = status.vramUsedGb;
    const total = status.vramTotalGb;
    element<HTMLElement>("runtime-vram").textContent = used !== null && total !== null
      ? `${used.toFixed(1)} / ${total.toFixed(1)} GB` : "— / — GB";
    const model = status.activeModel?.trim() || "—";
    const modelNode = element<HTMLElement>("runtime-model");
    modelNode.textContent = model;
    modelNode.title = model === "—" ? "Доступно при запуске WebUI с --api" : model;
  } catch {
    if (sequence !== runtimeSequence) return;
    element<HTMLElement>("runtime-dot").dataset.state = "offline";
    element<HTMLElement>("sidebar-status-dot").dataset.state = "offline";
    element<HTMLElement>("runtime-state").textContent = "Недоступен";
    element<HTMLElement>("sidebar-status-label").textContent = "Нет связи с WebUI";
    element<HTMLElement>("system-status").textContent = "Нет связи";
    element<HTMLElement>("runtime-port").textContent = "—";
    element<HTMLElement>("runtime-ping").textContent = "—";
    element<HTMLElement>("runtime-vram").textContent = "— / — GB";
    element<HTMLElement>("runtime-model").textContent = "—";
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
  previewGalleryImage = null;
  hideGalleryContextMenu();
  image.removeAttribute("src");
  imageDialog.showModal();
  try { image.src = await window.flow.getPreview(url); }
  catch (error) { imageDialog.close(); notify(message(error)); }
}

function hideGalleryContextMenu(): void {
  galleryContextMenu.hidden = true;
  contextGalleryImage = null;
  if (galleryContextMenu.parentElement !== document.body) document.body.append(galleryContextMenu);
}

function showGalleryContextMenu(item: GalleryImage, x: number, y: number): void {
  hideGalleryContextMenu();
  contextGalleryImage = item;
  const host = imageDialog.open ? imageDialog : document.body;
  host.append(galleryContextMenu);
  const bounds = imageDialog.open ? imageDialog.getBoundingClientRect() : null;
  const relativeX = x - (bounds?.left ?? 0);
  const relativeY = y - (bounds?.top ?? 0);
  galleryContextMenu.style.left = Math.max(8, relativeX) + "px";
  galleryContextMenu.style.top = Math.max(8, relativeY) + "px";
  galleryContextMenu.hidden = false;
  const width = bounds?.width ?? window.innerWidth;
  const height = bounds?.height ?? window.innerHeight;
  galleryContextMenu.style.left = Math.max(8, Math.min(relativeX, width - galleryContextMenu.offsetWidth - 8)) + "px";
  galleryContextMenu.style.top = Math.max(8, Math.min(relativeY, height - galleryContextMenu.offsetHeight - 8)) + "px";
}

function updateGallerySelection(): void {
  galleryGrid.classList.toggle("selecting", gallerySelectMode);
  gallerySelect.textContent = gallerySelectMode ? "Отмена" : "Выбрать";
  galleryDelete.hidden = !gallerySelectMode;
  galleryDelete.disabled = selectedGalleryPaths.size === 0;
  galleryDelete.textContent = "Удалить" + (selectedGalleryPaths.size ? " (" + selectedGalleryPaths.size + ")" : "");
  gallerySelectedCount.hidden = !gallerySelectMode;
  gallerySelectedCount.textContent = "Выбрано: " + selectedGalleryPaths.size;
  for (const button of galleryGrid.querySelectorAll<HTMLButtonElement>(".gallery-card")) {
    const selected = selectedGalleryPaths.has(button.dataset.path ?? "");
    button.classList.toggle("selected", gallerySelectMode && selected);
    if (gallerySelectMode) {
      button.setAttribute("aria-pressed", String(selected));
      button.setAttribute("aria-label", (selected ? "Снять выбор: " : "Выбрать: ") + (button.title || "изображение"));
    } else {
      button.removeAttribute("aria-pressed");
      button.setAttribute("aria-label", "Открыть изображение " + (button.title || ""));
    }
  }
}

function setGallerySelectMode(enabled: boolean): void {
  gallerySelectMode = enabled;
  selectedGalleryPaths.clear();
  hideGalleryContextMenu();
  updateGallerySelection();
}

function showGalleryImage(item: GalleryImage): void {
  const image = element<HTMLImageElement>("full-image");
  image.removeAttribute("src");
  image.alt = item.fileName;
  previewGalleryImage = item;
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
  button.dataset.path = item.path;
  button.setAttribute("aria-label", "Открыть изображение " + item.fileName);
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
  const selection = make("span", "gallery-selection", "✓");
  selection.setAttribute("aria-hidden", "true");
  button.append(image, caption, selection);
  button.addEventListener("click", () => {
    if (!gallerySelectMode) {
      showGalleryImage(item);
      return;
    }
    if (selectedGalleryPaths.has(item.path)) selectedGalleryPaths.delete(item.path);
    else if (selectedGalleryPaths.size < 500) selectedGalleryPaths.add(item.path);
    else { notify("За один раз можно удалить не более 500 изображений."); return; }
    updateGallerySelection();
  });
  button.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    const rect = button.getBoundingClientRect();
    showGalleryContextMenu(item, event.clientX || rect.left + 12, event.clientY || rect.top + 12);
  });
  return button;
}

async function refreshGallery(): Promise<void> {
  const sequence = ++gallerySequence;
  setGallerySelectMode(false);
  gallerySelect.disabled = true;
  galleryRefresh.disabled = true;
  galleryStatus.hidden = false;
  galleryStatus.textContent = "Ищем изображения WebUI…";
  galleryEmpty.hidden = true;
  galleryGrid.replaceChildren();
  try {
    const images = await window.flow.listGalleryImages();
    if (sequence !== gallerySequence) return;
    galleryImageCount = images.length;
    galleryCount.textContent = images.length + " изображений";
    galleryStatus.hidden = true;
    galleryEmpty.hidden = images.length > 0;
    const fragment = document.createDocumentFragment();
    for (const image of images) fragment.append(renderGalleryImage(image));
    galleryGrid.append(fragment);
  } catch (error) {
    if (sequence !== gallerySequence) return;
    galleryImageCount = 0;
    galleryStatus.textContent = "Не удалось загрузить галерею: " + message(error);
  } finally {
    if (sequence === gallerySequence) {
      gallerySelect.disabled = galleryImageCount === 0;
      galleryRefresh.disabled = false;
    }
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
      const option = make("option", "", file.baseModel ? `${file.name} · ${file.baseModel}` : file.name);
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
    const compatibility = make("small", "model-compatibility");
    const describe = (): void => {
      const base = model.files[Number(select.value)]?.baseModel ?? "";
      compatibility.textContent = base.toLowerCase().includes("sdxl") || base.toLowerCase().includes("illustrious")
        ? `${base}: Forge Neo рекомендуется для видеокарт с малой VRAM.`
        : base ? `${base}: проверьте поддержку архитектуры выбранным WebUI.` : "Архитектура файла не указана в Civitai.";
    };
    select.addEventListener("change", describe);
    describe();
    footer.append(select, button);
    body.append(compatibility);
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
  if (event.kind === "install" || event.kind === "launch" || event.kind === "xformers") {
    const label = event.kind === "install" ? "Установка" : event.kind === "xformers" ? "xformers" : "WebUI";
    queueLog(terminal, `[${label}] ${event.message}`);
  }
  if (event.kind === "xformers") {
    if (event.phase === "dependencies") setConsoleStatus("Установка xformers", "running");
    if (["done", "error", "cancelled"].includes(event.phase)) {
      setConsoleStatus(event.phase === "done" ? "xformers готов" : "Ошибка xformers", event.phase === "done" ? "done" : "error");
      installXformersButton.disabled = false;
      xformersStatus.textContent = event.message;
      if (event.phase !== "done") notify(event.message);
    }
    return;
  }
  if (event.kind === "install") {
    if (["checking", "cloning", "venv", "dependencies"].includes(event.phase)) setConsoleStatus("Установка", "running");
    if (["done", "error", "cancelled"].includes(event.phase)) {
      setConsoleStatus(event.phase === "done" ? "Установлено" : event.phase === "cancelled" ? "Отменено" : "Ошибка установки", event.phase === "done" ? "done" : "error");
      setInstalling(false);
      if (event.phase === "done") {
        notify(event.message);
        void window.flow.selectPackage(installingPackageId).then(async () => {
          activePackageId = installingPackageId;
          await downloadManager.restore();
          await refreshState();
          await firstRun.onInstallComplete();
          if (element<HTMLDialogElement>("first-run-dialog").open) setConsoleVisible(false);
        }).catch((error: unknown) => notify(message(error)));
      }
      else notify(event.message);
    }
    return;
  }
  if (event.kind === "launch") {
    if (event.phase === "model_error") {
      setConsoleStatus("Модель не загрузилась", "error");
      notify(event.message);
    }
    if (event.phase === "starting") {
      webuiRunning = true;
      updateConsoleControls();
      setConsoleStatus("Запуск WebUI", "running");
    }
    if (event.phase === "stabilizing") setConsoleStatus("Подготовка WebUI", "running");
    if (event.phase === "ready" && event.url) {
      setConsoleStatus("WebUI запущен", "running");
      void refreshState();
    }
    if (event.phase === "stopping") setConsoleStatus("Остановка WebUI", "running");
    if (event.phase === "done" || event.phase === "error") {
      stoppingWebui = false;
      webuiRunning = false;
      updateConsoleControls();
      setConsoleStatus(event.phase === "done" ? "WebUI остановлен" : "Ошибка WebUI", event.phase === "done" ? "done" : "error");
      if (event.phase === "error") notify(event.message);
      void refreshState();
    }
    return;
  }
  downloadManager.onTask(event);
  firstRun.onTask(event);
}

document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
  button.addEventListener("click", () => {
    const page = button.dataset.page;
    if (page === "home" || page === "gallery" || page === "models" || page === "settings") {
      setConsoleVisible(false);
      showPage(page);
    }
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
  else { installDialog.showModal(); void refreshPreflight(); }
});
installPackageType.addEventListener("change", () => {
  const importForge = installPackageType.value === "forge-neo-import";
  element<HTMLElement>("package-install-location").hidden = importForge;
  element<HTMLElement>("forge-install-location").hidden = !importForge;
  element<HTMLElement>("install-description").textContent = importForge
    ? "Подключите готовый Forge Neo из Stability Matrix. В Stability Matrix откройте папку пакета через меню «⋮» и остановите его перед запуском в Flow."
    : "Flow скачает Forge Neo, подготовит Python 3.13 в отдельном окружении и установит зависимости. Ход установки появится в консоли.";
  element<HTMLButtonElement>("start-install").textContent = importForge ? "Подключить Forge Neo" : "Начать установку";
  element<HTMLButtonElement>("start-install").disabled = !importForge;
  void refreshPreflight();
});
element<HTMLButtonElement>("browse-forge-dialog").addEventListener("click", async () => {
  try {
    const selected = await window.flow.browseInstallationDirectory(forgeDirInput.value || installationDir);
    if (selected) forgeDirInput.value = selected;
  } catch (error) { notify(message(error)); }
});
element<HTMLButtonElement>("close-install").addEventListener("click", () => installDialog.close());
element<HTMLButtonElement>("start-install").addEventListener("click", async () => {
  if (installing) return;
  if (installPackageType.value === "forge-neo-import") {
    if (!forgeDirInput.value.trim()) { notify("Выберите папку установленного Forge Neo."); return; }
    try {
      await window.flow.registerForgeNeo(forgeDirInput.value);
      activePackageId = "forge-neo";
      installDialog.close();
      notify("Forge Neo подключён к Flow.");
      await downloadManager.restore();
      await refreshState();
      await firstRun.onInstallComplete();
    } catch (error) { notify(message(error)); }
    return;
  }
  if (!installationDir) { notify("Сначала выберите папку установки."); return; }
  installingPackageId = "forge-neo";
  setInstalling(true);
  installDialog.close();
  setConsoleVisible(true);
  setConsoleStatus("Установка", "running");
  queueLog(terminal, `[Установка] Начало установки ${"Forge Neo"}...`);
  try {
    await window.flow.installForgeNeo(installationDir);
  }
  catch (error) {
    setInstalling(false);
    setConsoleStatus("Ошибка установки", "error");
    queueLog(terminal, `[Установка] ${message(error)}`);
    notify(message(error));
  }
});
consoleToggle.addEventListener("click", () => setConsoleVisible(!consolePanel.classList.contains("visible")));
consoleStop.addEventListener("click", () => void controlWebui(false));
consoleRestart.addEventListener("click", () => void controlWebui(true));
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
function applyMemoryPreset(vramFlag: "--lowvram" | "--medvram-sdxl", label: string): void {
  if (!webuiSettingsReady) return;
  webuiDraft = webuiDraft.filter((item) => !attentionFlags.has(webuiArgumentName(item)) && !vramFlags.has(webuiArgumentName(item)));
  webuiDraft.push(vramFlag, vramFlag === "--lowvram" ? "--opt-sdp-no-mem-attention" : "--opt-sub-quad-attention");
  sdxlMemoryProfile.checked = true;
  webuiSettingsDirty = true;
  renderWebuiArguments();
  setWebuiSettingsMessage(`${label} подготовлен. Сохраните настройки и перезапустите WebUI.`);
}
element<HTMLButtonElement>("apply-sdxl-profile").addEventListener("click", () => applyMemoryPreset("--lowvram", "Профиль 4 ГБ"));
element<HTMLButtonElement>("apply-sdxl-balanced").addEventListener("click", () => applyMemoryPreset("--medvram-sdxl", "Профиль SDXL"));
sdxlMemoryProfile.addEventListener("change", () => setWebuiSettingsMessage("Есть несохранённые изменения"));
element<HTMLButtonElement>("check-xformers").addEventListener("click", async () => {
  xformersStatus.textContent = "Проверка окружения…";
  try { xformersStatus.textContent = `xformers ${await window.flow.checkXformers()} доступен`; }
  catch (error) { xformersStatus.textContent = message(error); }
});
installXformersButton.addEventListener("click", async () => {
  if (webuiSettingsDirty) { setWebuiSettingsMessage("Сначала сохраните изменения аргументов запуска.", true); return; }
  installXformersButton.disabled = true;
  webuiSettingsDialog.close();
  setConsoleVisible(true);
  setConsoleStatus("Установка xformers", "running");
  try { await window.flow.installXformers(); }
  catch (error) {
    installXformersButton.disabled = false;
    setConsoleStatus("Ошибка xformers", "error");
    xformersStatus.textContent = message(error);
  }
});
element<HTMLButtonElement>("close-webui-settings").addEventListener("click", () => webuiSettingsDialog.close());
element<HTMLButtonElement>("cancel-webui-settings").addEventListener("click", () => webuiSettingsDialog.close());
webuiSettingsDialog.addEventListener("close", () => { ++webuiSettingsRequest; webuiSettingsReady = false; });
webuiSaveButton.addEventListener("click", async () => {
  if (!webuiSettingsReady) return;
  const request = webuiSettingsRequest;
  const settingsToSave = { arguments: [...webuiDraft], sdxlMemoryProfile: sdxlMemoryProfile.checked };
  webuiSaveButton.disabled = true;
  try {
    await window.flow.saveWebUISettings(settingsToSave);
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
gallerySelect.addEventListener("click", () => setGallerySelectMode(!gallerySelectMode));
galleryDelete.addEventListener("click", () => {
  if (selectedGalleryPaths.size === 0) return;
  element<HTMLElement>("gallery-delete-message").textContent =
    "Удалить " + selectedGalleryPaths.size + " изображений без возможности восстановления?";
  galleryDeleteDialog.showModal();
});
element<HTMLButtonElement>("cancel-gallery-delete").addEventListener("click", () => galleryDeleteDialog.close());
galleryDeleteConfirm.addEventListener("click", async () => {
  const paths = [...selectedGalleryPaths];
  if (paths.length === 0) { galleryDeleteDialog.close(); return; }
  galleryDeleteConfirm.disabled = true;
  try {
    const count = await window.flow.deleteGalleryImages(paths);
    notify("Удалено изображений: " + count);
  } catch (error) {
    notify("Не удалось удалить изображения: " + message(error));
  } finally {
    galleryDeleteConfirm.disabled = false;
    galleryDeleteDialog.close();
    await refreshGallery();
  }
});
element<HTMLButtonElement>("copy-gallery-image").addEventListener("click", async () => {
  const item = contextGalleryImage;
  hideGalleryContextMenu();
  if (!item) return;
  try {
    await window.flow.copyGalleryImage(item.path);
    notify(item.fileName.toLowerCase().endsWith(".png") ? "PNG скопирован с параметрами. Вставьте его в PNG info через Ctrl+V." : "Изображение скопировано. Вставьте его в PNG info через Ctrl+V.");
  } catch (error) {
    notify("Не удалось скопировать изображение: " + message(error));
  }
});
element<HTMLImageElement>("full-image").addEventListener("contextmenu", (event) => {
  if (!previewGalleryImage) return;
  event.preventDefault();
  showGalleryContextMenu(previewGalleryImage, event.clientX, event.clientY);
});
imageDialog.addEventListener("close", () => {
  previewGalleryImage = null;
  hideGalleryContextMenu();
});
document.addEventListener("pointerdown", (event) => {
  if (!galleryContextMenu.hidden && !galleryContextMenu.contains(event.target as Node)) hideGalleryContextMenu();
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") hideGalleryContextMenu();
});
window.addEventListener("scroll", hideGalleryContextMenu, true);
element<HTMLButtonElement>("search-button").addEventListener("click", () => void searchModels());
searchInput.addEventListener("input", () => {
  window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(() => void searchModels(), 400);
});
searchInput.addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); void searchModels(); } });
element<HTMLButtonElement>("browse-install-settings").addEventListener("click", () => void browseInstallationDir());
element<HTMLButtonElement>("browse-install-dialog").addEventListener("click", () => void browseInstallationDir());
element<HTMLButtonElement>("browse-models-library").addEventListener("click", () => void browseModelsLibrary());
element<HTMLButtonElement>("close-health").addEventListener("click", () => element<HTMLDialogElement>("health-dialog").close());
element<HTMLButtonElement>("health-close").addEventListener("click", () => element<HTMLDialogElement>("health-dialog").close());
element<HTMLButtonElement>("check-flow-update").addEventListener("click", async () => {
  const status = element<HTMLElement>("flow-update-status");
  status.textContent = "Проверяем выпуски…";
  try {
    const update = await window.flow.checkFlowUpdate();
    status.textContent = update.available ? `Доступна версия ${update.latestVersion} (установлена ${update.currentVersion}). Откройте «Выпуски» для установки.`
      : update.latestVersion ? `Установлена актуальная версия ${update.currentVersion}.` : "Опубликованных выпусков пока нет.";
  } catch (error) { status.textContent = `Не удалось проверить: ${message(error)}`; }
});
element<HTMLButtonElement>("open-flow-releases").addEventListener("click", () => void window.flow.openFlowReleases().catch((error: unknown) => notify(message(error))));
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
void window.flow.onTask(onTask).catch((error: unknown) => notify(message(error)));
const firstRun = new FirstRunController(
  flowApi,
  element<HTMLElement>("app-shell"),
  (file, type, name) => downloadManager.enqueue(file, type, name),
  () => { void refreshState(); },
  () => notify("Следующий шаг — установите WebUI через «Добавить пакет». Выбор модели откроется после установки.")
);
void firstRun.start().catch((error: unknown) => notify(message(error)));
void refreshState();
window.setInterval(() => { if (document.visibilityState === "visible") void refreshWebUIStatus(); }, 15_000);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") void refreshWebUIStatus(); });
void downloadManager.restore().catch((error: unknown) => notify(message(error)));
void window.flow.setPage("home").catch(() => { /* Discord may be unavailable. */ });

