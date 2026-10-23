import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppState, CheckReport, DownloadedFile, FlowAPI, FlowUpdate, GalleryImage, ModelFile, ModelSummary, PackageInfo, Page, StarterModel, TaskEvent, WebUISettings, WebUIStatus } from "./types";

// The UI only knows this typed contract. System access stays in Rust.
export const flowApi: FlowAPI = {
  getState: () => invoke<AppState>("get_state"),
  getWebUIStatus: () => invoke<WebUIStatus>("get_webui_status"),
  installForgeNeo: (installationDir: string) => invoke<void>("install_forge_neo_web_ui", { installationDir }),
  cancelInstall: () => invoke<void>("cancel_install"),
  launchWebUI: (packageId?: PackageInfo["id"]) => invoke<void>("launch_web_ui", { packageId }),
  registerForgeNeo: (directory: string) => invoke<void>("register_forge_neo", { directory }),
  selectPackage: (packageId: PackageInfo["id"]) => invoke<void>("select_package", { packageId }),
  stopWebUI: () => invoke<void>("stop_web_ui"),
  openWebUI: () => invoke<void>("open_web_ui"),
  getWebUIArguments: () => invoke<string[]>("get_webui_arguments"),
  saveWebUIArguments: (argumentsList: string[]) => invoke<string[]>("save_webui_arguments", { arguments: argumentsList }),
  getWebUISettings: () => invoke<WebUISettings>("get_webui_settings"),
  saveWebUISettings: (settings: WebUISettings) => invoke<WebUISettings>("save_webui_settings", { settings }),
  checkXformers: () => invoke<string>("check_xformers"),
  installXformers: () => invoke<void>("install_xformers"),
  listGalleryImages: () => invoke<GalleryImage[]>("list_gallery_images"),
  copyGalleryImage: (path: string) => invoke<void>("copy_gallery_image", { path }),
  deleteGalleryImages: (paths: string[]) => invoke<number>("delete_gallery_images", { paths }),
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
  getOnboardingStage: () => invoke<number>("get_onboarding_stage"),
  saveOnboardingToken: (token: string | null) => invoke<void>("save_onboarding_token", { token }),
  finishOnboarding: () => invoke<void>("finish_onboarding"),
  openCivitaiSite: () => invoke<void>("open_civitai_site"),
  saveInstallationDirectory: (directory: string) => invoke<void>("set_installation_dir", { directory }),
  saveModelsLibraryDirectory: (directory: string) => invoke<void>("set_models_library_dir", { directory }),
  getPreflight: (packageId: PackageInfo["id"]) => invoke<CheckReport>("get_preflight", { packageId }),
  checkPackage: (packageId: PackageInfo["id"]) => invoke<CheckReport>("check_package", { packageId }),
  repairPackage: (packageId: PackageInfo["id"]) => invoke<void>("repair_package", { packageId }),
  updatePackage: (packageId: PackageInfo["id"]) => invoke<void>("update_package", { packageId }),
  rollbackPackage: (packageId: PackageInfo["id"]) => invoke<void>("rollback_package", { packageId }),
  checkFlowUpdate: () => invoke<FlowUpdate>("check_flow_update"),
  openFlowReleases: () => invoke<void>("open_flow_releases"),
  linkPackageModels: (packageId: PackageInfo["id"]) => invoke<string>("link_package_models", { packageId }),
  setPage: (page: Page) => invoke<void>("set_page", { page }),
  onTask: async (callback: (event: TaskEvent) => void) => listen<TaskEvent>("flow:task", (event) => callback(event.payload)),
  browseInstallationDirectory: async (currentPath: string) => {
    // Tauri v2 returns a path or null for a single directory selection.
    const selected = await open({ directory: true, multiple: false, defaultPath: currentPath || undefined });
    return typeof selected === "string" ? selected : null;
  }
};
