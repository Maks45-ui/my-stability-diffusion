import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppState, DownloadedFile, FlowAPI, GalleryImage, ModelFile, ModelSummary, Page, StarterModel, TaskEvent, WebUISettings, WebUIStatus } from "./types";

// The UI only knows this typed contract. System access stays in Rust.
export const flowApi: FlowAPI = {
  getState: () => invoke<AppState>("get_state"),
  getWebUIStatus: () => invoke<WebUIStatus>("get_webui_status"),
  installWebUI: (installationDir: string) => invoke<void>("install_web_ui", { installationDir }),
  cancelInstall: () => invoke<void>("cancel_install"),
  launchWebUI: () => invoke<void>("launch_web_ui"),
  stopWebUI: () => invoke<void>("stop_web_ui"),
  openWebUI: () => invoke<void>("open_web_ui"),
  getWebUIArguments: () => invoke<string[]>("get_webui_arguments"),
  saveWebUIArguments: (argumentsList: string[]) => invoke<string[]>("save_webui_arguments", { arguments: argumentsList }),
  getWebUISettings: () => invoke<WebUISettings>("get_webui_settings"),
  saveWebUISettings: (settings: WebUISettings) => invoke<WebUISettings>("save_webui_settings", { settings }),
  checkXformers: () => invoke<string>("check_xformers"),
  installXformers: () => invoke<void>("install_xformers"),
  listGalleryImages: () => invoke<GalleryImage[]>("list_gallery_images"),
  searchModels: (query: string) => invoke<ModelSummary[]>("search_models", { query }),
  getStarterModel: (key: string) => invoke<StarterModel>("get_starter_model", { key }),
  downloadModel: (file: ModelFile, type: string, taskId: string, resume: boolean) => invoke<void>("download_model", { file, modelType: type, taskId, resume }),
  pauseDownload: () => invoke<void>("pause_download"),
  cancelDownload: () => invoke<void>("cancel_download"),
  listDownloads: () => invoke<DownloadedFile[]>("list_downloads"),
  deleteDownload: (modelType: string, fileName: string) => invoke<void>("delete_download", { modelType, fileName }),
  getPreview: (url: string) => invoke<string>("get_preview", { url }),
  saveCivitaiToken: (token: string) => invoke<void>("save_civitai_token", { token }),
  getOnboardingComplete: () => invoke<boolean>("get_onboarding_complete"),
  completeOnboarding: (token: string | null) => invoke<void>("complete_onboarding", { token }),
  getOnboardingStage: () => invoke<number>("get_onboarding_stage"),
  saveOnboardingToken: (token: string | null) => invoke<void>("save_onboarding_token", { token }),
  finishOnboarding: () => invoke<void>("finish_onboarding"),
  openCivitaiSite: () => invoke<void>("open_civitai_site"),
  saveInstallationDirectory: (directory: string) => invoke<void>("set_installation_dir", { directory }),
  setPage: (page: Page) => invoke<void>("set_page", { page }),
  onTask: async (callback: (event: TaskEvent) => void) => listen<TaskEvent>("flow:task", (event) => callback(event.payload)),
  browseInstallationDirectory: async (currentPath: string) => {
    // Tauri v2 returns a path or null for a single directory selection.
    const selected = await open({ directory: true, multiple: false, defaultPath: currentPath || undefined });
    return typeof selected === "string" ? selected : null;
  }
};
