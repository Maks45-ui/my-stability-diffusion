export type Page = "home" | "gallery" | "models" | "settings";
export type TaskPhase = "idle" | "checking" | "cloning" | "venv" | "dependencies" | "done" | "error" | "cancelled";

export interface PackageInfo {
  name: string;
  version: string;
  path: string;
  running: boolean;
}

export interface AppState {
  packages: PackageInfo[];
  hasCivitaiToken: boolean;
  modelsPath: string;
}

export interface ModelFile {
  name: string;
  downloadUrl: string;
  sizeKb: number;
}

export interface ModelSummary {
  id: number;
  name: string;
  type: string;
  images: string[];
  files: ModelFile[];
}

export interface TaskEvent {
  kind: "install" | "download";
  phase: TaskPhase;
  message: string;
  percent?: number;
  bytesPerSecond?: number;
}

export interface FlowAPI {
  getState(): Promise<AppState>;
  installWebUI(): Promise<void>;
  cancelInstall(): Promise<void>;
  launchWebUI(): Promise<void>;
  openPackageSettings(): Promise<void>;
  searchModels(query: string): Promise<ModelSummary[]>;
  downloadModel(file: ModelFile, type: string): Promise<void>;
  cancelDownload(): Promise<void>;
  getPreview(url: string): Promise<string>;
  saveCivitaiToken(token: string): Promise<void>;
  setPage(page: Page): Promise<void>;
  onTask(callback: (event: TaskEvent) => void): () => void;
}

declare global {
  interface Window {
    flow: FlowAPI;
  }
}

