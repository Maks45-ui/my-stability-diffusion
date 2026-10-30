export type Page = "home" | "gallery" | "models" | "settings";
export type TaskPhase = "idle" | "checking" | "cloning" | "venv" | "dependencies" | "verifying" | "starting" | "stabilizing" | "ready" | "running" | "stopping" | "log" | "model_error" | "paused" | "done" | "error" | "cancelled";

export interface PackageInfo {
  id: "automatic1111" | "forge-neo";
  name: string;
  version: string;
  path: string;
  running: boolean;
  stopping: boolean;
  stopped: boolean;
  readyUrl: string | null;
  active: boolean;
  managed: boolean;
  hasRollback: boolean;
}

export interface AppState {
  packages: PackageInfo[];
  hasCivitaiToken: boolean;
  modelsPath: string;
  installationDir: string;
  modelsLibraryDir: string;
}

export interface CheckEntry {
  label: string;
  value: string;
  level: "ok" | "warning" | "error" | "info";
}

export interface CheckReport {
  title: string;
  checks: CheckEntry[];
  canProceed: boolean;
}

export interface FlowUpdate {
  currentVersion: string;
  latestVersion: string | null;
  available: boolean;
  url: string;
}

export interface WebUIStatus {
  running: boolean;
  port: number | null;
  pingMs: number | null;
  vramUsedGb: number | null;
  vramTotalGb: number | null;
  activeModel: string | null;
}

export interface GalleryImage {
  path: string;
  fileName: string;
  kind: "txt2img" | "img2img";
  modifiedAt: number;
}

export interface GalleryImageDetails {
  prompt: string | null;
  negativePrompt: string | null;
  seed: string | null;
  steps: string | null;
  sampler: string | null;
  model: string | null;
  parameters: string | null;
}

export interface ModelFile {
  name: string;
  downloadUrl: string;
  sizeKb: number;
  baseModel?: string | null;
  sha256?: string | null;
}

export interface StarterModel {
  previewUrl: string | null;
  file: ModelFile;
}

export interface DownloadedFile {
  name: string;
  modelType: string;
  sizeBytes: number;
  downloadedBytes: number;
  status: "completed" | "paused";
  downloadUrl: string | null;
  sha256: string | null;
}

export interface ModelSummary {
  id: number;
  name: string;
  type: string;
  description: string | null;
  tags: string[];
  creator: string | null;
  downloadCount: number | null;
  favoriteCount: number | null;
  updatedAt: string | null;
  images: string[];
  files: ModelFile[];
}

export interface TaskEvent {
  kind: "install" | "launch" | "download" | "xformers";
  phase: TaskPhase;
  message: string;
  url?: string;
  percent?: number;
  bytesPerSecond?: number;
  taskId?: string;
  bytesDownloaded?: number;
  totalBytes?: number;
}

export interface WebUISettings {
  arguments: string[];
  sdxlMemoryProfile: boolean;
}

export interface FlowAPI {
  getState(): Promise<AppState>;
  getWebUIStatus(): Promise<WebUIStatus>;
  installForgeNeo(installationDir: string): Promise<void>;
  browseInstallationDirectory(currentPath: string): Promise<string | null>;
  saveInstallationDirectory(directory: string): Promise<void>;
  saveModelsLibraryDirectory(directory: string): Promise<void>;
  getPreflight(packageId: PackageInfo["id"]): Promise<CheckReport>;
  checkPackage(packageId: PackageInfo["id"]): Promise<CheckReport>;
  repairPackage(packageId: PackageInfo["id"]): Promise<void>;
  updatePackage(packageId: PackageInfo["id"]): Promise<void>;
  rollbackPackage(packageId: PackageInfo["id"]): Promise<void>;
  checkFlowUpdate(): Promise<FlowUpdate>;
  openFlowReleases(): Promise<void>;
  linkPackageModels(packageId: PackageInfo["id"]): Promise<string>;
  cancelInstall(): Promise<void>;
  launchWebUI(packageId?: PackageInfo["id"]): Promise<void>;
  registerForgeNeo(directory: string): Promise<void>;
  selectPackage(packageId: PackageInfo["id"]): Promise<void>;
  stopWebUI(): Promise<void>;
  openWebUI(): Promise<void>;
  getWebUIArguments(): Promise<string[]>;
  saveWebUIArguments(argumentsList: string[]): Promise<string[]>;
  getWebUISettings(): Promise<WebUISettings>;
  saveWebUISettings(settings: WebUISettings): Promise<WebUISettings>;
  checkXformers(): Promise<string>;
  installXformers(): Promise<void>;
  listGalleryImages(): Promise<GalleryImage[]>;
  getGalleryImageDetails(path: string): Promise<GalleryImageDetails>;
  copyGalleryImage(path: string): Promise<void>;
  deleteGalleryImages(paths: string[]): Promise<number>;
  searchModels(query: string, sort: "popular" | "newest" | "rated", modelType: string): Promise<ModelSummary[]>;
  getStarterModel(key: string): Promise<StarterModel>;
  downloadModel(file: ModelFile, type: string, taskId: string, resume: boolean): Promise<void>;
  pauseDownload(): Promise<void>;
  cancelDownload(): Promise<void>;
  listDownloads(): Promise<DownloadedFile[]>;
  deleteDownload(modelType: string, fileName: string): Promise<void>;
  getPreview(url: string): Promise<string>;
  saveCivitaiToken(token: string): Promise<void>;
  getOnboardingComplete(): Promise<boolean>;
  getOnboardingStage(): Promise<number>;
  saveOnboardingToken(token: string | null): Promise<void>;
  finishOnboarding(): Promise<void>;
  openCivitaiSite(): Promise<void>;
  setPage(page: Page): Promise<void>;
  onTask(callback: (event: TaskEvent) => void): Promise<() => void>;
}

export type Theme = "light" | "dark" | "system" | "pink" | "zakaz" | "crimson-moon" | "blueviolet-dusk" | "boykisser";
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

