"""Поиск моделей и потоковая загрузка файлов Civitai."""

from dataclasses import dataclass
import os
from pathlib import Path
import re
import tempfile
import time
from urllib.parse import urlparse

import requests


API_URL = "https://civitai.com/api/v1/models"
DEFAULT_API_KEY = "9b3da4cabd83c429ff171d87034e1fd4"
CHUNK_SIZE = 256 * 1024


class CivitaiError(Exception):
    """Ошибка API или загрузки, которую можно показать пользователю."""


class DownloadCancelled(CivitaiError):
    pass


@dataclass(frozen=True)
class ModelFile:
    name: str
    download_url: str
    model_type: str
    size_kb: float = 0


def models_directory():
    return Path(os.environ.get("LOCALAPPDATA") or Path.home()) / "Flow" / "models"


class CivitaiClient:
    def __init__(self, api_key=DEFAULT_API_KEY):
        self.headers = {
            "Authorization": f"Bearer {api_key}",
            "User-Agent": "Flow/1.0",
        }

    @staticmethod
    def _check_response(response):
        if response.status_code == 401:
            raise CivitaiError("Civitai отклонил API-ключ (401).")
        if response.status_code == 403:
            raise CivitaiError("Нет доступа к этой модели или файлу (403).")
        if response.status_code == 404:
            raise CivitaiError("Модель или файл не найдены (404).")
        if response.status_code == 429:
            raise CivitaiError("Civitai ограничил частоту запросов. Повторите позже.")
        if not response.ok:
            raise CivitaiError(f"Ошибка Civitai: HTTP {response.status_code}.")

    def search_models(self, query):
        query = query.strip()
        if not query:
            return []

        url = f"{API_URL}/{query}" if query.isdecimal() else API_URL
        params = None if query.isdecimal() else {"query": query, "limit": 10}
        try:
            response = requests.get(
                url, params=params, headers=self.headers, timeout=(10, 30)
            )
            if query.isdecimal() and response.status_code == 404:
                return []
            self._check_response(response)
            data = response.json()
        except requests.RequestException as exc:
            raise CivitaiError("Не удалось связаться с Civitai.") from exc
        except ValueError as exc:
            raise CivitaiError("Civitai вернул некорректный ответ.") from exc

        models = [data] if query.isdecimal() else data.get("items", [])
        if not isinstance(models, list):
            raise CivitaiError("Civitai вернул некорректный список моделей.")
        return models

    def download_file(self, model_file, progress, cancel_event):
        url = urlparse(model_file.download_url)
        if url.scheme != "https" or url.hostname != "civitai.com":
            raise CivitaiError("Недопустимая ссылка для скачивания модели.")

        folders = {
            "Checkpoint": "checkpoints",
            "LORA": "loras",
            "VAE": "vae",
            "TextualInversion": "embeddings",
            "Controlnet": "controlnet",
        }
        directory = models_directory() / folders.get(model_file.model_type, "other")
        directory.mkdir(parents=True, exist_ok=True)
        filename = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", model_file.name).strip(" .")
        if not filename:
            raise CivitaiError("У файла модели нет допустимого имени.")

        temporary_path = None
        try:
            with requests.get(
                model_file.download_url,
                headers=self.headers,
                stream=True,
                timeout=(10, 60),
            ) as response:
                self._check_response(response)
                content_type = response.headers.get("Content-Type", "").lower()
                if "text/html" in content_type or "application/json" in content_type:
                    raise CivitaiError("Civitai вернул страницу вместо файла модели.")

                size_header = response.headers.get("Content-Length", "")
                total = int(size_header) if size_header.isdecimal() else None
                if not total and model_file.size_kb:
                    total = round(model_file.size_kb * 1024)

                started = last_update = time.monotonic()
                downloaded = 0
                with tempfile.NamedTemporaryFile(
                    mode="wb", prefix="flow-", suffix=".part", dir=directory,
                    delete=False,
                ) as output:
                    temporary_path = Path(output.name)
                    for chunk in response.iter_content(chunk_size=CHUNK_SIZE):
                        if cancel_event.is_set():
                            raise DownloadCancelled("Загрузка отменена.")
                        if not chunk:
                            continue
                        output.write(chunk)
                        downloaded += len(chunk)
                        now = time.monotonic()
                        if now - last_update >= 0.2:
                            progress(downloaded, total, downloaded / max(now - started, 0.001))
                            last_update = now

                if cancel_event.is_set():
                    raise DownloadCancelled("Загрузка отменена.")
                if downloaded == 0:
                    raise CivitaiError("Получен пустой файл модели.")
                if size_header.isdecimal() and downloaded != int(size_header):
                    raise CivitaiError("Загрузка прервалась до получения всего файла.")

                target = directory / filename
                if target.exists():
                    base, suffix = target.stem, target.suffix
                    index = 2
                    while target.exists():
                        target = directory / f"{base} ({index}){suffix}"
                        index += 1
                temporary_path.replace(target)
                temporary_path = None
                progress(downloaded, total, downloaded / max(time.monotonic() - started, 0.001))
                return target
        except requests.RequestException as exc:
            raise CivitaiError("Соединение прервалось во время скачивания.") from exc
        finally:
            if temporary_path is not None:
                temporary_path.unlink(missing_ok=True)
