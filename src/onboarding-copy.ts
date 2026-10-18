// Keep first-run model text in a single UTF-8 TypeScript module. Vite bundles these
// strings instead of relying on text embedded in the HTML template.
export const starterKeys = ["indigo", "wai", "realistic"] as const;
export type StarterKey = (typeof starterKeys)[number];

interface StarterCopy {
  name: string;
  version: string;
  type: string;
  previewAlt: string;
}

export const starterCopy: Record<StarterKey, StarterCopy> = {
  indigo: {
    name: "Indigo Furry mix",
    version: "v120_hybrid",
    type: "STABLE DIFFUSION 1.x · CHECKPOINT",
    previewAlt: "Пример изображения Indigo Furry mix",
  },
  wai: {
    name: "WAI-illustrious-SDXL",
    version: "v17.0",
    type: "ILLUSTRIOUS · CHECKPOINT",
    previewAlt: "Пример изображения WAI-illustrious-SDXL",
  },
  realistic: {
    name: "Realistic Vision V6.0 B1",
    version: "V5.1 Hyper (VAE)",
    type: "STABLE DIFFUSION 1.x · CHECKPOINT",
    previewAlt: "Пример изображения Realistic Vision V6.0 B1",
  },
};

export const onboardingCopy = {
  keyTitle: "Перед тем как начать",
  keyIntro: "Подключите Civitai, чтобы искать и скачивать модели. Этот шаг можно выполнить позже в настройках.",
  modelTitle: "Выберите первую модель",
  modelIntro: "Выберите одну модель для папки WebUI. Этот шаг можно пропустить и скачать модель позже.",
  loading: "Загружаем сведения о модели…",
  download: "Скачать",
  retry: "Повторить",
  skipModels: "Пропустить и начать работу",
  queued: "Ожидает начала загрузки…",
  downloading: "Скачивается",
  downloaded: "Модель загружена. Открываем Flow…",
  paused: "Загрузка приостановлена. Продолжите в каталоге.",
  chooseAnother: "Можно выбрать другую модель или начать работу без загрузки.",
  alreadyQueued: "Эта модель уже есть в списке загрузок. Откройте каталог после завершения мастера.",
  downloadLocation: "Загрузка идёт в папку sd-webui/models/Stable-diffusion. Можно начать работу, пока она продолжается.",
  finishing: "Завершаем настройку…",
} as const;
