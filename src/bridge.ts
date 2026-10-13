import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppState, FlowAPI, ModelFile, ModelSummary, Page, TaskEvent } from "./types";

// The UI only knows this typed contract. System access stays in Rust.
export const flowApi: FlowAPI = {
  getState: () => invoke<AppState>("get_state"),
  installWebUI: (installationDir: string) => invoke<void>("install_web_ui", { installationDir }),
  cancelInstall: () => invoke<void>("cancel_install"),
  launchWebUI: () => invoke<void>("launch_web_ui"),
  openPackageSettings: () => invoke<void>("open_package_settings"),
  searchModels: (query: string) => invoke<ModelSummary[]>("search_models", { query }),
  downloadModel: (file: ModelFile, type: string) => invoke<void>("download_model", { file, modelType: type }),
  cancelDownload: () => invoke<void>("cancel_download"),
  getPreview: (url: string) => invoke<string>("get_preview", { url }),
  saveCivitaiToken: (token: string) => invoke<void>("save_civitai_token", { token }),
  saveInstallationDirectory: (directory: string) => invoke<void>("set_installation_dir", { directory }),
  setPage: (page: Page) => invoke<void>("set_page", { page }),
  onTask: async (callback: (event: TaskEvent) => void) => listen<TaskEvent>("flow:task", (event) => callback(event.payload)),
  browseInstallationDirectory: async (currentPath: string) => {
    // Tauri v2 returns a path or null for a single directory selection.
    const selected = await open({ directory: true, multiple: false, defaultPath: currentPath || undefined });
    return typeof selected === "string" ? selected : null;
  }
};
