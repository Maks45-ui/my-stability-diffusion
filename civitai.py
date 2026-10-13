"""Поиск моделей и потоковая загрузка файлов Civitai."""

from dataclasses import dataclass
from io import BytesIO
import os
from pathlib import Path
import re
import tempfile
import time
from urllib.parse import urlparse

import requests
from PIL import Image


API_URL = "https://civitai.com/api/v1/models"
DEFAULT_API_KEY = "9b3da4cabd83c429ff171d87034e1fd4"
CHUNK_SIZE = 256 * 1024
MAX_IMAGE_BYTES = 32 * 1024 * 1024


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


def model_images(model):
    """Все уникальные примеры из modelVersions[].images ответа API."""
    images = []
    seen = set()
    for version in model.get("modelVersions") or []:
        for image in version.get("images") or []:
            url = image.get("url") if isinstance(image, dict) else None
            if not url or url in seen or image.get("type") == "video":
                continue
            seen.add(url)
            images.append(url)
    return images


def sized_image_url(url, width):
    return re.sub(r"/width=\d+", f"/width={width}", url, count=1)


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
        is_id = query.isascii() and query.isdecimal()
        params = {"limit": 8}
        if is_id:
            params["ids"] = query
        elif query:
            params["query"] = query
        else:
            params["sort"] = "Most Downloaded"
        try:
            response = requests.get(
                API_URL, params=params, headers=self.headers, timeout=(10, 30)
            )
            self._check_response(response)
            data = response.json()
        except requests.RequestException as exc:
            raise CivitaiError("Не удалось связаться с Civitai.") from exc
        except ValueError as exc:
            raise CivitaiError("Civitai вернул некорректный ответ.") from exc

        models = data.get("items", []) if isinstance(data, dict) else None
        if not isinstance(models, list):
            raise CivitaiError("Civitai вернул некорректный список моделей.")
        return models

    @staticmethod
    def load_image(url, max_size):
        """Получить и уменьшить пример вне потока Tkinter, не раскрывая API-ключ CDN."""
        parsed = urlparse(url)
        if parsed.scheme != "https" or not (
            parsed.hostname == "civitai.com"
            or (parsed.hostname or "").endswith(".civitai.com")
        ):
            raise CivitaiError("Недопустимая ссылка на изображение.")

        image_url = sized_image_url(url, max_size[0])
        try:
            with requests.get(
                image_url, headers={"User-Agent": "Flow/1.0"},
                timeout=(8, 20), stream=True,
            ) as response:
                CivitaiClient._check_response(response)
                if not response.headers.get("Content-Type", "").lower().startswith("image/"):
                    raise CivitaiError("Сервер вернул не изображение.")
                data = BytesIO()
                for chunk in response.iter_content(chunk_size=CHUNK_SIZE):
                    data.write(chunk)
                    if data.tell() > MAX_IMAGE_BYTES:
                        raise CivitaiError("Изображение слишком большое.")
            data.seek(0)
            with Image.open(data) as source:
                picture = source.convert("RGB")
            picture.thumbnail(max_size, Image.Resampling.LANCZOS)
            return picture
        except requests.RequestException as exc:
            raise CivitaiError("Не удалось загрузить изображение.") from exc
        except (OSError, ValueError) as exc:
            raise CivitaiError("Не удалось открыть изображение.") from exc

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
