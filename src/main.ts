import { app, BrowserWindow, ipcMain, shell } from "electron";
import { access, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { createWriteStream } from "node:fs";
import { spawn, execFile, type ChildProcess } from "node:child_process";
import { connect, type Socket } from "node:net";
import { join, basename } from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";
import { promisify } from "node:util";
import type { AppState, ModelFile, ModelSummary, PackageInfo, Page, TaskEvent, TaskPhase } from "./types";

const execFileAsync = promisify(execFile);
const WEBUI_REPO = "https://github.com/AUTOMATIC1111/stable-diffusion-webui.git";
const SD_REPO = "https://github.com/w-e-w/stablediffusion.git";
const CIVITAI_API = "https://civitai.red/api/v1/models";
const CLIENT_ID = "1559517880825942036";
app.setName("Flow");
const PAGE_STATE: Record<Page, string> = {
  home: "Главная", gallery: "Просмотр галереи", models: "Загрузка моделей", settings: "В настройках"
};

let window: BrowserWindow | null = null;
let installAbort: AbortController | null = null;
let downloadAbort: AbortController | null = null;
let activeChild: ChildProcess | null = null;
let webuiChild: ChildProcess | null = null;

const dataDir = (): string => join(app.getPath("userData"), "Flow");
const packageDir = (): string => join(dataDir(), "packages", "sd-webui");
const modelsDir = (): string => join(dataDir(), "models");
const markerPath = (): string => join(packageDir(), ".flow-installed.json");
const settingsPath = (): string => join(dataDir(), "settings.json");
const venvPython = (): string => join(packageDir(), "venv", process.platform === "win32" ? "Scripts/python.exe" : "bin/python");

function emit(kind: TaskEvent["kind"], phase: TaskPhase, message: string, percent?: number, bytesPerSecond?: number): void {
  window?.webContents.send("flow:task", { kind, phase, message, percent, bytesPerSecond } satisfies TaskEvent);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; } catch { return false; }
}

async function readToken(): Promise<string> {
  if (process.env.CIVITAI_API_KEY) return process.env.CIVITAI_API_KEY;
  try {
    const value: unknown = JSON.parse(await readFile(settingsPath(), "utf8"));
    return isRecord(value) && typeof value.civitaiToken === "string" ? value.civitaiToken : "";
  } catch { return ""; }
}

async function packageInfo(): Promise<PackageInfo[]> {
  if (!(await exists(markerPath()))) return [];
  let version = "Установлен";
  try {
    const marker: unknown = JSON.parse(await readFile(markerPath(), "utf8"));
    if (isRecord(marker) && typeof marker.version === "string") version = marker.version;
  } catch { /* The marker still proves an earlier installation. */ }
  return [{ name: "AUTOMATIC1111 WebUI", version, path: packageDir(), running: webuiChild?.exitCode === null && webuiChild?.pid !== undefined }];
}

async function state(): Promise<AppState> {
  return { packages: await packageInfo(), hasCivitaiToken: Boolean(await readToken()), modelsPath: modelsDir() };
}

async function commandOutput(file: string, args: string[], cwd?: string): Promise<string> {
  const result = await execFileAsync(file, args, { cwd, timeout: 12_000, windowsHide: true });
  return result.stdout.trim();
}

async function findPython(): Promise<[string, string[]]> {
  const candidates: Array<[string, string[]]> = process.platform === "win32"
    ? [["py", ["-3.10"]], ["python", []], ["python3.10", []]]
    : [["python3.10", []], ["python3", []], ["python", []]];
  for (const [file, args] of candidates) {
    try {
      if (await commandOutput(file, [...args, "-c", "import sys; print('%d.%d' % sys.version_info[:2])"]) === "3.10") return [file, args];
    } catch { /* Try another executable. */ }
  }
  throw new Error("Не найден Python 3.10. Установите его и перезапустите Flow.");
}

async function run(file: string, args: string[], cwd: string, phase: TaskPhase, signal: AbortSignal): Promise<void> {
  if (signal.aborted) throw new Error("Установка отменена.");
  await new Promise<void>((resolve, reject) => {
    const child = spawn(file, args, {
      cwd, windowsHide: true, env: { ...process.env, PYTHONUNBUFFERED: "1", GIT_TERMINAL_PROMPT: "0", WEBUI_LAUNCH_LIVE_OUTPUT: "1", STABLE_DIFFUSION_REPO: SD_REPO },
      stdio: ["ignore", "pipe", "pipe"]
    });
    activeChild = child;
    const log = (data: Buffer): void => emit("install", phase, data.toString("utf8"));
    child.stdout?.on("data", log);
    child.stderr?.on("data", log);
    child.once("error", reject);
    child.once("close", (code) => {
      activeChild = null;
      if (signal.aborted) reject(new Error("Установка отменена."));
      else if (code === 0) resolve();
      else reject(new Error(`Команда завершилась с кодом ${code ?? "?"}.`));
    });
  });
}

async function install(signal: AbortSignal): Promise<void> {
  emit("install", "checking", "Проверка Git и Python 3.10…");
  await commandOutput("git", ["--version"]);
  const [python, pythonArgs] = await findPython();
  const directory = packageDir();
  await mkdir(join(dataDir(), "packages"), { recursive: true });

  emit("install", "cloning", "Подготовка репозитория AUTOMATIC1111…");
  if (await exists(directory)) {
    if (!(await exists(join(directory, ".git")))) throw new Error("Папка пакета занята другим содержимым.");
    const remote = await commandOutput("git", ["-C", directory, "remote", "get-url", "origin"]);
    if (remote.replace(/\.git\/?$/, "").toLowerCase() !== WEBUI_REPO.replace(/\.git$/, "").toLowerCase()) {
      throw new Error("В папке пакета находится другой Git-репозиторий.");
    }
    emit("install", "cloning", "Существующий репозиторий найден, продолжаю.");
  } else {
    await run("git", ["clone", "--progress", WEBUI_REPO, directory], join(dataDir(), "packages"), "cloning", signal);
  }

  if (!(await exists(venvPython()))) {
    emit("install", "venv", "Создание виртуального окружения…");
    await run(python, [...pythonArgs, "-m", "venv", join(directory, "venv")], directory, "venv", signal);
  }
  if (!(await exists(join(directory, "launch.py")))) throw new Error("Файл launch.py не найден.");
  emit("install", "dependencies", "Установка зависимостей WebUI. Это может занять несколько минут…");
  await run(venvPython(), ["-u", "launch.py", "--exit", "--skip-torch-cuda-test"], directory, "dependencies", signal);
  if (signal.aborted) throw new Error("Установка отменена.");
  let version = "Установлен";
  try { version = await commandOutput("git", ["-C", directory, "describe", "--tags", "--always"]); } catch { /* Version is optional. */ }
  if (process.platform === "win32") {
    const settings = join(directory, "webui.settings.bat");
    if (!(await exists(settings))) await writeFile(settings, `@echo off\r\nset PYTHON="${venvPython()}"\r\nset STABLE_DIFFUSION_REPO=${SD_REPO}\r\n`, "utf8");
  }
  await writeFile(markerPath(), JSON.stringify({ name: "AUTOMATIC1111 WebUI", version }), "utf8");
  emit("install", "done", "AUTOMATIC1111 WebUI установлен.");
}

function killTree(child: ChildProcess | null): void {
  if (!child?.pid) return;
  if (process.platform === "win32") spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true });
  else child.kill("SIGTERM");
}

function apiHeaders(token: string): HeadersInit {
  return token ? { Authorization: `Bearer ${token}`, "User-Agent": "Flow/1.0" } : { "User-Agent": "Flow/1.0" };
}

function checkResponse(response: Response): void {
  if (response.ok) return;
  const messages: Record<number, string> = {
    401: "Civitai отклонил API-ключ.", 403: "Нет доступа к этой модели.",
    404: "Модель или файл не найдены.", 429: "Слишком много запросов. Повторите позже."
  };
  throw new Error(messages[response.status] ?? `Ошибка Civitai: HTTP ${response.status}.`);
}

function modelFromApi(raw: unknown): ModelSummary | null {
  if (!isRecord(raw) || typeof raw.id !== "number" || typeof raw.name !== "string") return null;
  const images: string[] = [];
  const files: ModelFile[] = [];
  const versions = Array.isArray(raw.modelVersions) ? raw.modelVersions : [];
  for (const version of versions) {
    if (!isRecord(version)) continue;
    for (const image of Array.isArray(version.images) ? version.images : []) {
      if (isRecord(image) && typeof image.url === "string" && image.type !== "video" && !images.includes(image.url)) images.push(image.url);
    }
    for (const file of Array.isArray(version.files) ? version.files : []) {
      if (isRecord(file) && typeof file.name === "string" && typeof file.downloadUrl === "string") {
        files.push({ name: file.name, downloadUrl: file.downloadUrl, sizeKb: typeof file.sizeKB === "number" ? file.sizeKB : 0 });
      }
    }
  }
  return { id: raw.id, name: raw.name, type: typeof raw.type === "string" ? raw.type : "Model", images, files };
}

async function searchModels(query: string): Promise<ModelSummary[]> {
  const url = new URL(CIVITAI_API);
  url.searchParams.set("limit", "12");
  url.searchParams.set("nsfw", "true");
  if (/^\d+$/.test(query.trim())) url.searchParams.set("ids", query.trim());
  else if (query.trim()) url.searchParams.set("query", query.trim());
  else url.searchParams.set("sort", "Most Downloaded");
  const response = await fetch(url, { headers: apiHeaders(await readToken()), signal: AbortSignal.timeout(30_000) });
  checkResponse(response);
  const body: unknown = await response.json();
  if (!isRecord(body) || !Array.isArray(body.items)) throw new Error("Civitai вернул некорректный список моделей.");
  return body.items.map(modelFromApi).filter((model): model is ModelSummary => model !== null);
}

function safeDownloadUrl(raw: string): URL {
  const url = new URL(raw);
  if (url.protocol !== "https:" || !["civitai.com", "civitai.red"].includes(url.hostname) || !url.pathname.startsWith("/api/download/models/")) {
    throw new Error("Недопустимая ссылка для скачивания модели.");
  }
  url.hostname = "civitai.red";
  return url;
}

async function downloadModel(file: ModelFile, type: string, signal: AbortSignal): Promise<void> {
  const url = safeDownloadUrl(file.downloadUrl);
  const cleanType = /^[\w -]{1,40}$/.test(type) ? type.replace(/\s+/g, "-") : "Other";
  const folder = join(modelsDir(), cleanType);
  await mkdir(folder, { recursive: true });
  const cleanName = basename(file.name).replace(/[<>:"/\\|?*\x00-\x1f]/g, "_");
  if (!cleanName || cleanName === "." || cleanName === "..") throw new Error("Некорректное имя файла.");
  let destination = join(folder, cleanName);
  for (let index = 1; await exists(destination); index++) destination = join(folder, `${index}-${cleanName}`);
  const partial = `${destination}.part`;
  const response = await fetch(url, { headers: apiHeaders(await readToken()), signal });
  checkResponse(response);
  if (!response.body) throw new Error("Civitai не вернул данные файла.");
  const total = Number(response.headers.get("content-length")) || 0;
  let received = 0;
  const started = Date.now();
  const stream = Readable.fromWeb(response.body as import("node:stream/web").ReadableStream<Uint8Array>);
  stream.on("data", (chunk: Buffer) => {
    received += chunk.length;
    const speed = received / Math.max(1, (Date.now() - started) / 1000);
    emit("download", "dependencies", cleanName, total ? Math.min(100, Math.round(received / total * 100)) : undefined, speed);
  });
  try {
    await pipeline(stream, createWriteStream(partial, { flags: "wx" }), { signal });
    await rename(partial, destination);
    emit("download", "done", `Сохранено: ${destination}`, 100);
  } catch (error) {
    await rm(partial, { force: true });
    throw error;
  }
}

function safeImageUrl(raw: string): URL {
  const url = new URL(raw);
  const host = url.hostname.toLowerCase();
  if (url.protocol !== "https:" || !["civitai.com", "civitai.red"].some((domain) => host === domain || host.endsWith(`.${domain}`))) {
    throw new Error("Недопустимая ссылка на изображение.");
  }
  return url;
}

async function preview(raw: string): Promise<string> {
  const url = safeImageUrl(raw);
  const response = await fetch(url, { headers: apiHeaders(await readToken()), signal: AbortSignal.timeout(20_000) });
  checkResponse(response);
  const type = response.headers.get("content-type")?.split(";")[0] ?? "";
  if (!["image/jpeg", "image/png", "image/webp", "image/gif"].includes(type)) throw new Error("Превью недоступно.");
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > 12 * 1024 * 1024) throw new Error("Изображение слишком большое.");
  return `data:${type};base64,${bytes.toString("base64")}`;
}

class DiscordPresence {
  private socket: Socket | null = null;
  private buffer = Buffer.alloc(0);
  private ready = false;
  private page: Page = "home";
  private readonly started = Math.floor(Date.now() / 1000);
  private retry: NodeJS.Timeout | null = null;

  start(): void { this.connectAt(0); }
  setPage(page: Page): void { this.page = page; this.update(); }
  stop(): void { if (this.retry) clearTimeout(this.retry); this.retry = null; this.socket?.destroy(); }

  private connectAt(index: number): void {
    if (index > 9) { this.retry = setTimeout(() => this.connectAt(0), 30_000); this.retry.unref(); return; }
    const base = process.platform === "win32" ? "\\\\.\\pipe\\" : `${process.env.XDG_RUNTIME_DIR ?? "/tmp"}/`;
    const socket = connect(`${base}discord-ipc-${index}`);
    socket.once("connect", () => {
      this.socket = socket;
      this.buffer = Buffer.alloc(0);
      this.frame(0, { v: 1, client_id: CLIENT_ID });
    });
    socket.on("data", (part: Buffer) => {
      this.buffer = Buffer.concat([this.buffer, part]);
      while (this.buffer.length >= 8) {
        const size = this.buffer.readUInt32LE(4);
        if (size > 1_000_000) { socket.destroy(); return; }
        if (this.buffer.length < 8 + size) break;
        const payload = this.buffer.subarray(8, 8 + size);
        this.buffer = this.buffer.subarray(8 + size);
        try {
          const message: unknown = JSON.parse(payload.toString("utf8"));
          if (isRecord(message) && message.evt === "READY") { this.ready = true; this.update(); }
        } catch { /* Ignore malformed Discord responses. */ }
      }
    });
    socket.once("error", () => { if (this.socket !== socket) this.connectAt(index + 1); });
    socket.once("close", () => {
      if (this.socket === socket) {
        this.socket = null; this.ready = false;
        this.retry = setTimeout(() => this.connectAt(0), 30_000);
        this.retry.unref();
      }
    });
  }

  private frame(op: number, value: object): void {
    if (!this.socket) return;
    const payload = Buffer.from(JSON.stringify(value));
    const header = Buffer.alloc(8);
    header.writeUInt32LE(op, 0);
    header.writeUInt32LE(payload.length, 4);
    this.socket.write(Buffer.concat([header, payload]));
  }

  private update(): void {
    if (!this.ready) return;
    this.frame(1, {
      cmd: "SET_ACTIVITY", args: {
        pid: process.pid,
        activity: {
          ...(this.page === "settings" ? { details: "Управление Flow" } : {}),
          state: PAGE_STATE[this.page], timestamps: { start: this.started }
        }
      }, nonce: `${Date.now()}`
    });
  }
}

const discord = new DiscordPresence();

function createWindow(): void {
  window = new BrowserWindow({
    width: 1280, height: 800, minWidth: 920, minHeight: 600,
    title: "Flow", backgroundColor: "#121820",
    icon: join(__dirname, "assets", "logo.png"),
    webPreferences: { preload: join(__dirname, "preload.js"), contextIsolation: true, nodeIntegration: false, sandbox: true }
  });
  void window.loadFile(join(__dirname, "index.html"));
  window.on("closed", () => { window = null; });
}

app.whenReady().then(() => {
  createWindow();
  discord.start();
  app.on("activate", () => { if (BrowserWindow.getAllWindows().length === 0) createWindow(); });
});
app.on("window-all-closed", () => { if (process.platform !== "darwin") app.quit(); });
app.on("before-quit", () => { discord.stop(); installAbort?.abort(); downloadAbort?.abort(); killTree(activeChild); });

ipcMain.handle("flow:state", state);
ipcMain.handle("flow:install", () => {
  if (installAbort) throw new Error("Установка уже выполняется.");
  const controller = new AbortController();
  installAbort = controller;
  void install(controller.signal).catch((error: unknown) => {
    emit("install", controller.signal.aborted ? "cancelled" : "error", error instanceof Error ? error.message : "Неизвестная ошибка установки.");
  }).finally(() => { installAbort = null; });
});
ipcMain.handle("flow:cancel-install", () => { installAbort?.abort(); killTree(activeChild); });
ipcMain.handle("flow:launch", async () => {
  if (!(await exists(markerPath()))) throw new Error("Сначала установите пакет.");
  if (webuiChild?.exitCode === null) return;
  const script = join(packageDir(), process.platform === "win32" ? "webui-user.bat" : "webui.sh");
  if (!(await exists(script))) throw new Error("Файл запуска WebUI не найден.");
  const child = process.platform === "win32"
    ? spawn("cmd.exe", ["/c", script], { cwd: packageDir(), detached: true, stdio: "ignore", windowsHide: false, env: { ...process.env, STABLE_DIFFUSION_REPO: SD_REPO } })
    : spawn("bash", [script], { cwd: packageDir(), detached: true, stdio: "ignore", env: { ...process.env, STABLE_DIFFUSION_REPO: SD_REPO } });
  webuiChild = child;
  try {
    await new Promise<void>((resolve, reject) => { child.once("spawn", resolve); child.once("error", reject); });
    child.unref();
  } catch (error) { webuiChild = null; throw error; }
});
ipcMain.handle("flow:package-settings", async () => {
  if (!(await exists(markerPath()))) throw new Error("Сначала установите пакет.");
  if (process.platform === "win32") {
    const file = join(packageDir(), "webui.settings.bat");
    const editor = spawn("notepad.exe", [file], { detached: true, stdio: "ignore" });
    await new Promise<void>((resolve, reject) => { editor.once("spawn", resolve); editor.once("error", reject); });
    editor.unref();
    return;
  }
  const target = packageDir();
  const error = await shell.openPath(target);
  if (error) throw new Error(error);
});
ipcMain.handle("flow:search", (_event, query: unknown) => searchModels(typeof query === "string" ? query.slice(0, 120) : ""));
ipcMain.handle("flow:download", (_event, raw: unknown, type: unknown) => {
  if (downloadAbort) throw new Error("Другая модель уже загружается.");
  if (!isRecord(raw) || typeof raw.name !== "string" || typeof raw.downloadUrl !== "string") throw new Error("Некорректный файл модели.");
  const file: ModelFile = { name: raw.name, downloadUrl: raw.downloadUrl, sizeKb: typeof raw.sizeKb === "number" ? raw.sizeKb : 0 };
  const controller = new AbortController();
  downloadAbort = controller;
  void downloadModel(file, typeof type === "string" ? type : "Other", controller.signal).catch((error: unknown) => {
    emit("download", controller.signal.aborted ? "cancelled" : "error", error instanceof Error ? error.message : "Ошибка загрузки.");
  }).finally(() => { downloadAbort = null; });
});
ipcMain.handle("flow:cancel-download", () => { downloadAbort?.abort(); });
ipcMain.handle("flow:preview", (_event, url: unknown) => {
  if (typeof url !== "string") throw new Error("Некорректная ссылка на изображение.");
  return preview(url);
});
ipcMain.handle("flow:token", async (_event, value: unknown) => {
  if (typeof value !== "string" || value.length > 512) throw new Error("Некорректный API-ключ.");
  await mkdir(dataDir(), { recursive: true });
  await writeFile(settingsPath(), JSON.stringify({ civitaiToken: value.trim() }), { encoding: "utf8", mode: 0o600 });
});
ipcMain.handle("flow:page", (_event, value: unknown) => {
  if (value === "home" || value === "gallery" || value === "models" || value === "settings") discord.setPage(value);
});

