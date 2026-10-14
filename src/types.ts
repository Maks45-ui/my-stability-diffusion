export type Page = "home" | "gallery" | "models" | "settings";
export type TaskPhase = "idle" | "checking" | "cloning" | "venv" | "dependencies" | "running" | "stopping" | "log" | "paused" | "done" | "error" | "cancelled";

export interface PackageInfo {
  name: string;
  version: string;
  path: string;
  running: boolean;
  stopping: boolean;
  stopped: boolean;
}

export interface AppState {
  packages: PackageInfo[];
  hasCivitaiToken: boolean;
  modelsPath: string;
  installationDir: string;
}

export interface GalleryImage {
  path: string;
  fileName: string;
  kind: "txt2img" | "img2img";
  modifiedAt: number;
}

export interface ModelFile {
  name: string;
  downloadUrl: string;
  sizeKb: number;
}

export interface DownloadedFile {
  name: string;
  modelType: string;
  sizeBytes: number;
  downloadedBytes: number;
  status: "completed" | "paused";
  downloadUrl: string | null;
}

export interface ModelSummary {
  id: number;
  name: string;
  type: string;
  images: string[];
  files: ModelFile[];
}

export interface TaskEvent {
  kind: "install" | "launch" | "download";
  phase: TaskPhase;
  message: string;
  percent?: number;
  bytesPerSecond?: number;
  taskId?: string;
  bytesDownloaded?: number;
  totalBytes?: number;
}

export interface FlowAPI {
  getState(): Promise<AppState>;
  installWebUI(installationDir: string): Promise<void>;
  browseInstallationDirectory(currentPath: string): Promise<string | null>;
  saveInstallationDirectory(directory: string): Promise<void>;
  cancelInstall(): Promise<void>;
  launchWebUI(): Promise<void>;
  stopWebUI(): Promise<void>;
  getWebUIArguments(): Promise<string[]>;
  saveWebUIArguments(argumentsList: string[]): Promise<string[]>;
  listGalleryImages(): Promise<GalleryImage[]>;
  searchModels(query: string): Promise<ModelSummary[]>;
  downloadModel(file: ModelFile, type: string, taskId: string, resume: boolean): Promise<void>;
  pauseDownload(): Promise<void>;
  cancelDownload(): Promise<void>;
  listDownloads(): Promise<DownloadedFile[]>;
  deleteDownload(modelType: string, fileName: string): Promise<void>;
  getPreview(url: string): Promise<string>;
  saveCivitaiToken(token: string): Promise<void>;
  setPage(page: Page): Promise<void>;
  onTask(callback: (event: TaskEvent) => void): Promise<() => void>;
}

export type Theme = "light" | "dark" | "system" | "zakaz" | "crimson-moon" | "blueviolet-dusk";
export type ResolvedTheme = Exclude<Theme, "system">;

export interface ThemeState {
  selected: Theme;
  resolved: ResolvedTheme;
}

declare global {
  interface Window {
    flow: FlowAPI;
  }
}
