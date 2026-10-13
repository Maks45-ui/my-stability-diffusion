import { contextBridge, ipcRenderer } from "electron";
import type { FlowAPI, ModelFile, Page, TaskEvent } from "./types";

const api: FlowAPI = {
  getState: () => ipcRenderer.invoke("flow:state"),
  installWebUI: () => ipcRenderer.invoke("flow:install"),
  cancelInstall: () => ipcRenderer.invoke("flow:cancel-install"),
  launchWebUI: () => ipcRenderer.invoke("flow:launch"),
  openPackageSettings: () => ipcRenderer.invoke("flow:package-settings"),
  searchModels: (query: string) => ipcRenderer.invoke("flow:search", query),
  downloadModel: (file: ModelFile, type: string) => ipcRenderer.invoke("flow:download", file, type),
  cancelDownload: () => ipcRenderer.invoke("flow:cancel-download"),
  getPreview: (url: string) => ipcRenderer.invoke("flow:preview", url),
  saveCivitaiToken: (token: string) => ipcRenderer.invoke("flow:token", token),
  setPage: (page: Page) => ipcRenderer.invoke("flow:page", page),
  onTask: (callback: (event: TaskEvent) => void) => {
    const listener = (_event: Electron.IpcRendererEvent, data: TaskEvent): void => callback(data);
    ipcRenderer.on("flow:task", listener);
    return () => ipcRenderer.removeListener("flow:task", listener);
  }
};

contextBridge.exposeInMainWorld("flow", api);

