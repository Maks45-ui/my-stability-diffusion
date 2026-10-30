import { flowApi } from "./bridge";
import { convertFileSrc } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
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
const gallerySearch = element<HTMLInputElement>("gallery-search");
const galleryFavoritesToggle = element<HTMLButtonElement>("gallery-favorites-toggle");
const galleryImageInfo = element<HTMLElement>("gallery-image-info");
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
const homeRecentGrid = element<HTMLElement>("home-recent-grid");
const homeActivityList = element<HTMLOListElement>("home-activity-list");
const inspectorImage = element<HTMLImageElement>("inspector-image");
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
let catalogModels: ModelSummary[] = [];
let activeModelFilter = "all";
let searchSequence = 0;
let gallerySequence = 0;
let homeRecentSequence = 0;
let inspectorSequence = 0;
let inspectedGalleryImage: GalleryImage | null = null;

let galleryItems: GalleryImage[] = [];
let showOnlyFavorites = false;
let galleryKind: GalleryImage["kind"] | "all" = "all";
function loadGalleryFavorites(): Set<string> {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem("flow.gallery.favorites") ?? "[]");
    return new Set(Array.isArray(stored) ? stored.filter((value): value is string => typeof value === "string") : []);
  } catch { return new Set<string>(); }
}
const galleryFavorites = loadGalleryFavorites();
let gallerySelectMode = false;
const selectedGalleryPaths = new Set<string>();
let pendingGalleryDeletePaths: string[] = [];
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
let webuiStartedAt: number | null = null;
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
  if (memoryCard) {
    const forge = activePackageId === "forge-neo";
    memoryCard.querySelectorAll<HTMLElement>(".webui-memory-presets, .webui-memory-toggle, .webui-xformers, .webui-memory-a1111").forEach((item) => { item.hidden = forge; });
    const advice = element<HTMLElement>("webui-memory-advice");
    advice.textContent = forge
      ? "Forge Neo управляет выгрузкой моделей самостоятельно. На 4 ГБ VRAM начинайте с 768 × 768 и увеличивайте размер постепенно."
      : "Определяю доступную видеопамять…";
    if (!forge) void window.flow.getPreflight("automatic1111").then((report) => {
      if (!webuiSettingsDialog.open || activePackageId === "forge-neo") return;
      const gpu = report.checks.find((check) => check.label === "Видеокарта");
      const mib = gpu?.value.match(/(\d+) МиБ VRAM/);
      const amount = mib ? Number(mib[1]) : null;
      advice.textContent = amount === null ? "VRAM не определена. Выберите профиль вручную по характеристикам видеокарты."
        : amount <= 5120 ? `Обнаружено ${(amount / 1024).toFixed(1)} ГБ VRAM. Рекомендуется «Профиль 4 ГБ» и перезапуск WebUI.`
        : amount <= 8192 ? `Обнаружено ${(amount / 1024).toFixed(1)} ГБ VRAM. Для SDXL начните со сбалансированного профиля.`
        : `Обнаружено ${(amount / 1024).toFixed(1)} ГБ VRAM. Дополнительные ограничения памяти обычно не нужны.`;
    }).catch(() => { advice.textContent = "Не удалось определить VRAM. Профиль можно выбрать вручную."; });
  }
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
  document.body.dataset.page = page;
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
  if (page === "home") void refreshHomeRecent();
}

function renderPackages(state: AppState): void {
  packageList.replaceChildren();
  const count = state.packages.length;
  const activePackage = state.packages.find((item) => item.active) ?? state.packages[0];
  activePackageId = activePackage?.id ?? "automatic1111";
  element<HTMLElement>("runtime-package").textContent = activePackage?.name ?? "Пакет не выбран";
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
  for (const item of [...state.packages].sort((left, right) => Number(right.active) - Number(left.active))) {
    const card = make("article", "package-card glass-surface");
    card.dataset.active = String(item.active);
    const icon = make("div", "package-icon");
    icon.innerHTML = '<svg viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linejoin="round" aria-hidden="true"><path d="M32 5 55 18v28L32 59 9 46V18L32 5Z"/><path d="m9 18 23 14 23-14M32 32v27"/></svg>';
    const details = make("div", "package-details");
    const titleRow = make("div", "package-title-row");
    titleRow.append(make("h3", "", item.name));
    details.append(titleRow, make("p", "package-description", item.id === "forge-neo"
      ? "Универсальный пакет для генерации изображений с поддержкой SDXL, Flux и расширений."
      : "Пакет Stable Diffusion WebUI для генерации изображений и работы с моделями."));
    const tags = make("div", "package-tags");
    for (const tag of (item.id === "forge-neo" ? ["SDXL", "Flux", "Плагины", item.version] : ["WebUI", item.version])) tags.append(make("span", "", tag));
    details.append(tags, make("small", "package-path", item.path));
    if (item.readyUrl) details.append(make("small", "package-url", item.readyUrl));
    const badge = make("span", "package-state", item.stopping ? "Останавливается" : item.readyUrl ? "Запущен" : item.running ? "Запускается" : item.stopped ? "Остановлен" : "Готов");
    badge.dataset.state = item.stopping ? "stopping" : item.readyUrl ? "running" : item.running ? "starting" : item.stopped ? "stopped" : "ready";
    titleRow.append(badge);
    card.append(icon, details);
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
    const more = make("details", "package-more");
    const moreSummary = make("summary", "secondary-button", "•••");
    moreSummary.setAttribute("aria-label", "Дополнительные действия с пакетом");
    more.append(moreSummary, settings);
    actions.append(launch, stop, open, more);
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
    more.append(linkModels);
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
      more.append(check, repair, update);
      if (item.hasRollback) {
        const rollback = make("button", "secondary-button", "Откатить");
        rollback.type = "button";
        rollback.disabled = item.running || installing;
        rollback.addEventListener("click", () => void runPackageMaintenance(item, "rollback"));
        more.append(rollback);
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
    if (!status.running) webuiStartedAt = null;
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
    element<HTMLElement>("home-vram").textContent = used !== null && total !== null
      ? `${used.toFixed(1)} / ${total.toFixed(1)} ГБ` : "— / — ГБ";
    element<HTMLElement>("home-vram-fill").style.width = used !== null && total !== null && total > 0
      ? `${Math.min(100, Math.round(used / total * 100))}%` : "0%";
    element<HTMLElement>("home-port").textContent = status.port === null ? "—" : `:${status.port}`;
    updateHomeUptime();
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
    element<HTMLElement>("home-vram").textContent = "— / — ГБ";
    element<HTMLElement>("home-vram-fill").style.width = "0%";
    element<HTMLElement>("home-port").textContent = "—";
  }
}

function updateHomeUptime(): void {
  if (!webuiStartedAt) { element<HTMLElement>("home-uptime").textContent = "—"; return; }
  const minutes = Math.max(0, Math.floor((Date.now() - webuiStartedAt) / 60_000));
  element<HTMLElement>("home-uptime").textContent = minutes >= 60
    ? `${Math.floor(minutes / 60)} ч ${minutes % 60} мин` : `${minutes} мин`;
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
  galleryImageInfo.hidden = true;
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
  element<HTMLButtonElement>("favorite-gallery-image").textContent =
    galleryFavorites.has(item.path) ? "★ Убрать из избранного" : "☆ В избранное";
  const width = bounds?.width ?? window.innerWidth;
  const height = bounds?.height ?? window.innerHeight;
  galleryContextMenu.style.left = Math.max(8, Math.min(relativeX, width - galleryContextMenu.offsetWidth - 8)) + "px";
  galleryContextMenu.style.top = Math.max(8, Math.min(relativeY, height - galleryContextMenu.offsetHeight - 8)) + "px";
}

function renderHomeRecent(images: GalleryImage[]): void {
  homeRecentGrid.replaceChildren();
  element<HTMLElement>("home-generated-count").textContent = String(images.length);
  if (images.length === 0) {
    homeRecentGrid.append(make("p", "home-placeholder", "Изображения появятся после генерации."));
    return;
  }
  for (const item of images.slice(0, 6)) {
    const button = make("button", "home-recent-image");
    button.type = "button";
    button.title = item.fileName;
    button.setAttribute("aria-label", "Открыть " + item.fileName);
    const image = make("img");
    image.alt = item.fileName;
    image.loading = "lazy";
    image.src = convertFileSrc(item.path);
    const caption = make("span", "home-recent-caption");
    const dimensions = make("span", "", "—");
    image.addEventListener("load", () => { dimensions.textContent = `${image.naturalWidth} × ${image.naturalHeight}`; });
    const age = make("small", "", item.modifiedAt ? new Date(item.modifiedAt).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "");
    caption.append(dimensions, age);
    button.append(image, caption);
    button.addEventListener("click", () => showGalleryImage(item));
    homeRecentGrid.append(button);
  }
}

async function refreshHomeRecent(): Promise<void> {
  const sequence = ++homeRecentSequence;
  try {
    const images = await window.flow.listGalleryImages();
    if (sequence === homeRecentSequence) renderHomeRecent(images);
  } catch {
    if (sequence === homeRecentSequence) renderHomeRecent([]);
  }
}

function updateInspectorFavorite(): void {
  const button = element<HTMLButtonElement>("inspector-favorite");
  button.textContent = inspectedGalleryImage && galleryFavorites.has(inspectedGalleryImage.path)
    ? "★" : "☆";
  button.setAttribute("aria-label", inspectedGalleryImage && galleryFavorites.has(inspectedGalleryImage.path)
    ? "Убрать из избранного" : "В избранное");
}

function inspectGalleryImage(item: GalleryImage | null): void {
  const sequence = ++inspectorSequence;
  inspectedGalleryImage = item;
  const hasImage = item !== null;
  for (const id of ["inspector-open", "inspector-copy", "inspector-favorite", "inspector-delete"]) {
    element<HTMLButtonElement>(id).disabled = !hasImage;
  }
  updateInspectorFavorite();
  element<HTMLElement>("inspector-placeholder").hidden = hasImage;
  inspectorImage.hidden = !hasImage;
  element<HTMLElement>("inspector-name").textContent = item?.fileName ?? "—";
  element<HTMLElement>("inspector-status").textContent = hasImage ? "Читаем параметры изображения…" : "Параметры PNG появятся здесь.";
  for (const id of ["model", "size", "seed", "steps", "sampler", "prompt", "negative"]) {
    element<HTMLElement>("inspector-" + id).textContent = "—";
  }
  element<HTMLElement>("inspector-file-name").textContent = item?.fileName ?? "—";
  element<HTMLElement>("inspector-file-kind").textContent = item?.kind ?? "—";
  element<HTMLElement>("inspector-file-date").textContent = item?.modifiedAt ? new Date(item.modifiedAt).toLocaleString("ru-RU") : "—";
  element<HTMLElement>("inspector-file-path").textContent = item?.path ?? "—";
  galleryGrid.querySelectorAll<HTMLElement>(".gallery-card").forEach((card) => card.classList.toggle("inspected", item !== null && card.dataset.path === item.path));
  if (!item) { inspectorImage.removeAttribute("src"); return; }
  inspectorImage.onload = () => {
    if (sequence === inspectorSequence) element<HTMLElement>("inspector-size").textContent = `${inspectorImage.naturalWidth} × ${inspectorImage.naturalHeight}`;
  };
  inspectorImage.onerror = () => {
    if (sequence === inspectorSequence) element<HTMLElement>("inspector-status").textContent = "Превью недоступно.";
  };
  inspectorImage.src = convertFileSrc(item.path);
  void window.flow.getGalleryImageDetails(item.path).then((details) => {
    if (sequence !== inspectorSequence) return;
    element<HTMLElement>("inspector-status").textContent = details.parameters ? "Параметры из PNG" : "В файле нет параметров генерации PNG.";
    element<HTMLElement>("inspector-model").textContent = details.model ?? "—";
    element<HTMLElement>("inspector-seed").textContent = details.seed ?? "—";
    element<HTMLElement>("inspector-steps").textContent = details.steps ?? "—";
    element<HTMLElement>("inspector-sampler").textContent = details.sampler ?? "—";
    element<HTMLElement>("inspector-prompt").textContent = details.prompt ?? "—";
    element<HTMLElement>("inspector-negative").textContent = details.negativePrompt ?? "—";
  }).catch((error: unknown) => {
    if (sequence === inspectorSequence) element<HTMLElement>("inspector-status").textContent = "Параметры недоступны: " + message(error);
  });
}

function renderGalleryFiltered(): void {
  const query = gallerySearch.value.trim().toLocaleLowerCase("ru-RU");
  const images = galleryItems.filter((item) =>
    (!showOnlyFavorites || galleryFavorites.has(item.path))
    && (galleryKind === "all" || item.kind === galleryKind)
    && (!query || item.fileName.toLocaleLowerCase("ru-RU").includes(query)));
  const fragment = document.createDocumentFragment();
  for (const image of images) fragment.append(renderGalleryImage(image));
  galleryGrid.replaceChildren(fragment);
  if (!images.some((item) => item.path === inspectedGalleryImage?.path)) inspectGalleryImage(images[0] ?? null);
  else galleryGrid.querySelector<HTMLElement>(`.gallery-card[data-path="${CSS.escape(inspectedGalleryImage?.path ?? "")}"]`)?.classList.add("inspected");
  galleryCount.textContent = galleryItems.length + " изображений";
  galleryEmpty.hidden = galleryItems.length > 0;
  galleryStatus.hidden = images.length > 0 || galleryItems.length === 0;
  if (galleryItems.length > 0 && images.length === 0) galleryStatus.textContent = "По выбранным фильтрам изображений нет.";
  gallerySelect.disabled = images.length === 0;
  updateGallerySelection();
}

function toggleGalleryFavorite(item: GalleryImage): void {
  if (galleryFavorites.has(item.path)) galleryFavorites.delete(item.path);
  else galleryFavorites.add(item.path);
  try { localStorage.setItem("flow.gallery.favorites", JSON.stringify([...galleryFavorites])); }
  catch { notify("Не удалось сохранить избранное."); }
  renderGalleryFiltered();
  updateInspectorFavorite();
}

function showGalleryDetails(item: GalleryImage): void {
  galleryImageInfo.hidden = false;
  element<HTMLElement>("gallery-info-status").textContent = "Загрузка параметров…";
  for (const id of ["prompt", "negative", "model", "seed", "steps", "sampler", "raw"]) {
    element<HTMLElement>("gallery-info-" + id).textContent = "—";
  }
  void window.flow.getGalleryImageDetails(item.path).then((details) => {
    if (!imageDialog.open || previewGalleryImage?.path !== item.path) return;
    element<HTMLElement>("gallery-info-status").textContent =
      details.parameters ? "Параметры из PNG" : "В файле нет параметров генерации PNG.";
    element<HTMLElement>("gallery-info-prompt").textContent = details.prompt ?? "—";
    element<HTMLElement>("gallery-info-negative").textContent = details.negativePrompt ?? "—";
    element<HTMLElement>("gallery-info-model").textContent = details.model ?? "—";
    element<HTMLElement>("gallery-info-seed").textContent = details.seed ?? "—";
    element<HTMLElement>("gallery-info-steps").textContent = details.steps ?? "—";
    element<HTMLElement>("gallery-info-sampler").textContent = details.sampler ?? "—";
    element<HTMLElement>("gallery-info-raw").textContent = details.parameters ?? "—";
  }).catch((error: unknown) => {
    if (imageDialog.open && previewGalleryImage?.path === item.path)
      element<HTMLElement>("gallery-info-status").textContent = "Ошибка чтения параметров: " + message(error);
  });
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
  inspectGalleryImage(item);
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
  showGalleryDetails(item);
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
  const favorite = make("span", "gallery-favorite", "★");
  favorite.setAttribute("aria-hidden", "true");
  favorite.hidden = !galleryFavorites.has(item.path);
  button.append(image, caption, selection, favorite);
  button.addEventListener("click", () => {
    if (!gallerySelectMode) {
      inspectGalleryImage(item);
      return;
    }
    if (selectedGalleryPaths.has(item.path)) selectedGalleryPaths.delete(item.path);
    else if (selectedGalleryPaths.size < 500) selectedGalleryPaths.add(item.path);
    else { notify("За один раз можно удалить не более 500 изображений."); return; }
    updateGallerySelection();
  });
  button.addEventListener("dblclick", () => { if (!gallerySelectMode) showGalleryImage(item); });
  button.addEventListener("focus", () => inspectGalleryImage(item));
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

    galleryItems = images;
    renderHomeRecent(images);
    renderGalleryFiltered();
  } catch (error) {
    if (sequence !== gallerySequence) return;

    galleryItems = [];
    inspectGalleryImage(null);
    galleryStatus.textContent = "Не удалось загрузить галерею: " + message(error);
  } finally {
    if (sequence === gallerySequence) {
      gallerySelect.disabled = galleryGrid.childElementCount === 0;
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
  if (model.images.length > 1) gallery.append(make("span", "model-gallery-count", `${model.images.length} фото · прокрутите`));
  const body = make("div", "model-body");
  const head = make("div", "model-head");
  const title = make("h2", "", model.name);
  const sizeLabel = make("span", "model-file-size");
  head.append(title, sizeLabel);
  const tags = make("div", "model-tags");
  tags.append(make("span", "model-type", model.type === "Checkpoint" ? "Чекпоинт" : model.type));
  const baseModel = model.files[0]?.baseModel;
  if (baseModel) tags.append(make("span", "", baseModel));
  for (const tag of model.tags.filter((tag) => tag.toLowerCase() !== baseModel?.toLowerCase()).slice(0, 2)) tags.append(make("span", "", tag));
  const plainDescription = (model.description ?? "").replace(/<[^>]*>/g, " ")
    .replace(/&nbsp;/gi, " ").replace(/&amp;/gi, "&").replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'").replace(/\s+/g, " ").trim();
  const description = make("p", "model-description", plainDescription.slice(0, 360)
    || `Модель ${model.type}. Просмотрите примеры и выберите файл для загрузки.`);
  const metadata = make("div", "model-metadata");
  if (model.downloadCount !== null) metadata.append(make("span", "", `↓ ${new Intl.NumberFormat("ru-RU", { notation: "compact" }).format(model.downloadCount)}`));
  if (model.favoriteCount !== null) metadata.append(make("span", "", `♡ ${new Intl.NumberFormat("ru-RU", { notation: "compact" }).format(model.favoriteCount)}`));
  if (model.creator) metadata.append(make("span", "", `♙ ${model.creator}`));
  if (model.updatedAt) {
    const date = new Date(model.updatedAt);
    if (!Number.isNaN(date.getTime())) metadata.append(make("span", "", `▣ ${date.toLocaleDateString("ru-RU")}`));
  }
  body.append(head, tags, description, metadata);
  const footer = make("div", "model-footer");
  if (model.files.length) {
    const select = make("select", "file-select");
    select.setAttribute("aria-label", `Файл модели ${model.name}`);
    for (const [index, file] of model.files.entries()) {
      const option = make("option", "", file.baseModel ? `${file.name} · ${file.baseModel}` : file.name);
      option.value = String(index);
      select.append(option);
    }
    const button = make("button", "download-button", "↓ Скачать");
    button.type = "button";
    const supportedFolder = /^(checkpoint)$/i.test(model.type) ? "Stable-diffusion"
      : /^(lora|locon|lycoris|dora)$/i.test(model.type) ? "Lora"
      : /^vae$/i.test(model.type) ? "VAE" : null;
    button.disabled = supportedFolder === null;
    if (!supportedFolder) button.title = "Автоматическая установка этого типа модели пока не поддерживается.";
    else button.title = "Папка назначения: models/" + supportedFolder;
    button.addEventListener("click", () => {
      const file = model.files[Number(select.value)];
      if (!file) return;
      downloadManager.enqueue(file, model.type, model.name, model.images[0] ?? null);
    });
    const compatibility = make("small", "model-compatibility");
    const describe = (): void => {
      const file = model.files[Number(select.value)];
      const base = file?.baseModel ?? "";
      sizeLabel.textContent = file?.sizeKb ? `${(file.sizeKb / 1024 / 1024).toFixed(2)} ГБ` : "Размер неизвестен";
      compatibility.textContent = supportedFolder === null
        ? "Этот тип пока нельзя установить автоматически."
        : (base.toLowerCase().includes("sdxl") || base.toLowerCase().includes("illustrious")
          ? `${base}: Forge Neo рекомендуется для видеокарт с малой VRAM.`
          : base ? `${base}: проверьте поддержку архитектуры выбранным WebUI.` : "Архитектура файла не указана в Civitai.")
          + " · Папка: models/" + supportedFolder;
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

function renderCatalog(): void {
  const filtered = activeModelFilter === "all" ? catalogModels
    : catalogModels.filter((model) => activeModelFilter === "lora"
      ? /^(lora|locon|lycoris|dora)$/i.test(model.type)
      : model.type.toLowerCase() === activeModelFilter);
  modelGrid.replaceChildren(...filtered.map(renderModel));
  catalogStatus.hidden = filtered.length > 0;
  if (!filtered.length) catalogStatus.textContent = catalogModels.length > 0
    ? "В этой категории моделей нет. Выберите другой тип или измените запрос."
    : "Модели не найдены. Попробуйте другой запрос.";
}

async function searchModels(): Promise<void> {
  const sequence = ++searchSequence;
  catalogModels = [];
  catalogStatus.textContent = "Загружаем модели…";
  catalogStatus.hidden = false;
  modelGrid.replaceChildren();
  try {
    const sort = element<HTMLSelectElement>("model-sort").value as "popular" | "newest" | "rated";
    const models = await window.flow.searchModels(searchInput.value, sort, activeModelFilter);
    if (sequence !== searchSequence) return;
    catalogModels = models;
    renderCatalog();
  } catch (error) {
    if (sequence !== searchSequence) return;
    catalogStatus.textContent = message(error);
  }
}

function onTask(event: TaskEvent): void {
  if (["starting", "cloning", "dependencies", "ready", "done", "error", "cancelled"].includes(event.phase)) {
    const entry = make("li", "", event.message.slice(0, 180));
    entry.dataset.phase = event.phase;
    const time = make("time", "", new Date().toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" }));
    entry.prepend(time);
    if (homeActivityList.firstElementChild?.textContent === "События установки и запуска появятся здесь.") homeActivityList.replaceChildren();
    homeActivityList.prepend(entry);
    while (homeActivityList.children.length > 5) homeActivityList.lastElementChild?.remove();
  }
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
      webuiStartedAt = Date.now();
      updateHomeUptime();
      setConsoleStatus("WebUI запущен", "running");
      void refreshState();
    }
    if (event.phase === "stopping") setConsoleStatus("Остановка WebUI", "running");
    if (event.phase === "done" || event.phase === "error") {
      webuiStartedAt = null;
      updateHomeUptime();
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
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
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
gallerySearch.addEventListener("input", () => { setGallerySelectMode(false); renderGalleryFiltered(); });
function updateGalleryKindButtons(): void {
  document.querySelectorAll<HTMLButtonElement>(".gallery-kind-filter").forEach((button) => {
    const active = !showOnlyFavorites && button.dataset.galleryKind === galleryKind;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  });
  galleryFavoritesToggle.classList.toggle("active", showOnlyFavorites);
  galleryFavoritesToggle.setAttribute("aria-pressed", String(showOnlyFavorites));
}
document.querySelectorAll<HTMLButtonElement>(".gallery-kind-filter").forEach((button) => button.addEventListener("click", () => {
  galleryKind = button.dataset.galleryKind === "txt2img" ? "txt2img" : button.dataset.galleryKind === "img2img" ? "img2img" : "all";
  showOnlyFavorites = false;
  updateGalleryKindButtons();
  setGallerySelectMode(false);
  renderGalleryFiltered();
}));
galleryFavoritesToggle.addEventListener("click", () => {
  showOnlyFavorites = !showOnlyFavorites;
  galleryKind = "all";
  updateGalleryKindButtons();
  setGallerySelectMode(false);
  renderGalleryFiltered();
});
element<HTMLButtonElement>("favorite-gallery-image").addEventListener("click", () => {
  const item = contextGalleryImage;
  hideGalleryContextMenu();
  if (item) toggleGalleryFavorite(item);
});
gallerySelect.addEventListener("click", () => setGallerySelectMode(!gallerySelectMode));
galleryDelete.addEventListener("click", () => {
  if (selectedGalleryPaths.size === 0) return;
  pendingGalleryDeletePaths = [...selectedGalleryPaths];
  element<HTMLElement>("gallery-delete-message").textContent =
    "Удалить " + pendingGalleryDeletePaths.length + " изображений без возможности восстановления?";
  galleryDeleteDialog.showModal();
});
element<HTMLButtonElement>("cancel-gallery-delete").addEventListener("click", () => galleryDeleteDialog.close());
galleryDeleteConfirm.addEventListener("click", async () => {
  const paths = [...pendingGalleryDeletePaths];
  if (paths.length === 0) { galleryDeleteDialog.close(); return; }
  galleryDeleteConfirm.disabled = true;
  try {
    const count = await window.flow.deleteGalleryImages(paths);
    notify("Удалено изображений: " + count);
  } catch (error) {
    notify("Не удалось удалить изображения: " + message(error));
  } finally {
    pendingGalleryDeletePaths = [];
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
document.querySelectorAll<HTMLButtonElement>(".model-filter").forEach((button) => {
  button.addEventListener("click", () => {
    activeModelFilter = button.dataset.modelType ?? "all";
    document.querySelectorAll<HTMLButtonElement>(".model-filter").forEach((item) => {
      const active = item === button;
      item.classList.toggle("active", active);
      item.setAttribute("aria-pressed", String(active));
    });
    if (hasLoadedCatalog) void searchModels();
  });
});
element<HTMLSelectElement>("model-sort").addEventListener("change", () => { if (hasLoadedCatalog) void searchModels(); });
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
let pendingFlowUpdate: Update | null = null;
const installFlowUpdateButton = element<HTMLButtonElement>("install-flow-update");
const flowUpdateStatus = element<HTMLElement>("flow-update-status");
element<HTMLButtonElement>("check-flow-update").addEventListener("click", async () => {
  flowUpdateStatus.textContent = "Проверяю подписанные обновления…";
  installFlowUpdateButton.hidden = true;
  pendingFlowUpdate = null;
  try {
    const update = await check();
    pendingFlowUpdate = update;
    if (update) {
      flowUpdateStatus.textContent = `Доступна версия ${update.version}. Установка потребует перезапуска Flow.`;
      installFlowUpdateButton.hidden = false;
    } else flowUpdateStatus.textContent = "Установлена актуальная версия Flow.";
  } catch (error) {
    flowUpdateStatus.textContent = `Проверка недоступна: ${message(error)}`;
  }
});
installFlowUpdateButton.addEventListener("click", async () => {
  const update = pendingFlowUpdate;
  if (!update) return;
  if (installing || webuiRunning) {
    flowUpdateStatus.textContent = "Перед обновлением завершите установку и остановите WebUI.";
    return;
  }
  installFlowUpdateButton.disabled = true;
  flowUpdateStatus.textContent = "Скачивание подписанного обновления…";
  let received = 0;
  let total = 0;
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") total = event.data.contentLength ?? 0;
      if (event.event === "Progress") received += event.data.chunkLength;
      if (event.event === "Finished") { flowUpdateStatus.textContent = "Обновление проверено и установлено. Перезапускаю Flow…"; return; }
      if (total > 0) flowUpdateStatus.textContent = `Скачивание обновления: ${Math.min(100, Math.floor(received / total * 100))}%`;
    });
    await relaunch();
  } catch (error) {
    flowUpdateStatus.textContent = `Обновление не удалось: ${message(error)}`;
    installFlowUpdateButton.disabled = false;
  }
});
element<HTMLButtonElement>("open-flow-releases").addEventListener("click", () => void window.flow.openFlowReleases().catch((error: unknown) => notify(message(error))));
element<HTMLButtonElement>("open-gallery-home").addEventListener("click", () => showPage("gallery"));
element<HTMLButtonElement>("home-show-logs").addEventListener("click", () => setConsoleVisible(true));
for (const tab of ["parameters", "file"] as const) {
  element<HTMLButtonElement>(`inspector-tab-${tab}`).addEventListener("click", () => {
    for (const name of ["parameters", "file"] as const) {
      const active = name === tab;
      const button = element<HTMLButtonElement>(`inspector-tab-${name}`);
      button.classList.toggle("active", active);
      button.setAttribute("aria-selected", String(active));
      element<HTMLElement>(`inspector-${name}`).hidden = !active;
    }
  });
}
element<HTMLButtonElement>("inspector-open").addEventListener("click", () => {
  if (inspectedGalleryImage) showGalleryImage(inspectedGalleryImage);
});
element<HTMLButtonElement>("inspector-copy").addEventListener("click", async () => {
  if (!inspectedGalleryImage) return;
  try { await window.flow.copyGalleryImage(inspectedGalleryImage.path); notify("Изображение скопировано. Вставьте его через Ctrl+V."); }
  catch (error) { notify("Не удалось скопировать изображение: " + message(error)); }
});
element<HTMLButtonElement>("inspector-favorite").addEventListener("click", () => {
  if (inspectedGalleryImage) toggleGalleryFavorite(inspectedGalleryImage);
});
element<HTMLButtonElement>("inspector-delete").addEventListener("click", () => {
  if (!inspectedGalleryImage) return;
  pendingGalleryDeletePaths = [inspectedGalleryImage.path];
  element<HTMLElement>("gallery-delete-message").textContent =
    `Удалить ${inspectedGalleryImage.fileName} без возможности восстановления?`;
  galleryDeleteDialog.showModal();
});
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
void refreshHomeRecent();
document.body.dataset.page = "home";
window.setInterval(updateHomeUptime, 60_000);
window.setInterval(() => { if (document.visibilityState === "visible") void refreshWebUIStatus(); }, 15_000);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") void refreshWebUIStatus(); });
void downloadManager.restore().catch((error: unknown) => notify(message(error)));
void window.flow.setPage("home").catch(() => { /* Discord may be unavailable. */ });


