"""Install and launch the AUTOMATIC1111 WebUI package for Flow."""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import threading


PACKAGE_NAME = "AUTOMATIC1111 WebUI"
REPOSITORY_URL = "https://github.com/AUTOMATIC1111/stable-diffusion-webui.git"
PACKAGE_DIR = "sd-webui"
MARKER_NAME = ".flow-installed.json"


class InstallationError(RuntimeError):
    pass


class InstallationCancelled(InstallationError):
    pass


def packages_root():
    """Keep downloads outside PyInstaller's temporary extraction directory."""
    if os.name == "nt":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
        return base / "Flow" / "packages"
    return Path.home() / ".local" / "share" / "Flow" / "packages"


def package_path():
    return packages_root() / PACKAGE_DIR


def venv_python(directory):
    if os.name == "nt":
        return directory / "venv" / "Scripts" / "python.exe"
    return directory / "venv" / "bin" / "python"


def is_installed():
    directory = package_path()
    return (directory / MARKER_NAME).is_file() and venv_python(directory).is_file()


def installed_packages():
    return [(PACKAGE_NAME, package_path())] if is_installed() else []


def find_git():
    git = shutil.which("git")
    if not git:
        raise InstallationError("Git не найден. Установите Git и перезапустите Flow.")
    return git


def find_python():
    """WebUI's Windows dependencies target Python 3.10; never use Flow.exe."""
    candidates = []
    if os.name == "nt" and shutil.which("py"):
        candidates.append([shutil.which("py"), "-3.10"])
    for name in ("python3.10", "python"):
        executable = shutil.which(name)
        if executable:
            candidates.append([executable])

    for candidate in candidates:
        try:
            result = subprocess.run(
                candidate + ["-c", "import sys; print('%d.%d' % sys.version_info[:2])"],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            if result.returncode == 0 and result.stdout.strip() == "3.10":
                return candidate
        except (OSError, subprocess.TimeoutExpired):
            continue
    raise InstallationError(
        "Python 3.10 не найден. Установите Python 3.10 (рекомендуется 3.10.6) "
        "и перезапустите Flow."
    )


class WebUIInstaller:
    def __init__(self, log):
        self.log = log
        self._cancelled = threading.Event()
        self._process = None
        self._process_lock = threading.Lock()

    def cancel(self):
        self._cancelled.set()
        with self._process_lock:
            process = self._process
        if process is not None and process.poll() is None:
            try:
                if os.name == "nt":
                    subprocess.run(
                        ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                        capture_output=True,
                        check=False,
                    )
                else:
                    os.killpg(process.pid, signal.SIGTERM)
            except OSError:
                pass

    def _check_cancelled(self):
        if self._cancelled.is_set():
            raise InstallationCancelled("Установка отменена.")

    def _run(self, command, cwd=None):
        self._check_cancelled()
        flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
        environment = os.environ.copy()
        environment["PYTHONUNBUFFERED"] = "1"
        try:
            process = subprocess.Popen(
                [str(part) for part in command],
                cwd=cwd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                bufsize=1,
                env=environment,
                creationflags=flags,
                start_new_session=os.name != "nt",
            )
        except OSError as exc:
            raise InstallationError(f"Не удалось запустить команду: {exc}") from exc

        with self._process_lock:
            self._process = process
        try:
            for line in process.stdout:
                self.log(line.rstrip("\r\n"))
                self._check_cancelled()
            code = process.wait()
            self._check_cancelled()
            if code:
                raise InstallationError(f"Команда завершилась с кодом {code}.")
        finally:
            with self._process_lock:
                self._process = None
            if process.poll() is None:
                process.terminate()
                process.wait()
            process.stdout.close()

    def install(self):
        self.log("Проверка Git и Python 3.10…")
        git = find_git()
        python = find_python()
        self._check_cancelled()
        directory = package_path()
        directory.parent.mkdir(parents=True, exist_ok=True)

        if directory.exists():
            if not (directory / ".git").is_dir():
                raise InstallationError(
                    f"Папка {directory} уже существует и не является репозиторием WebUI. "
                    "Переместите её и повторите установку."
                )
            remote = subprocess.run(
                [git, "-C", str(directory), "remote", "get-url", "origin"],
                capture_output=True, text=True, check=False,
            )
            if remote.returncode or remote.stdout.strip().rstrip("/").removesuffix(".git").lower() != REPOSITORY_URL.removesuffix(".git").lower():
                raise InstallationError("В папке пакета находится другой Git-репозиторий.")
            self.log("Найден существующий репозиторий WebUI; продолжаю установку.")
        else:
            self.log(f"Клонирование {REPOSITORY_URL} в {directory}…")
            self._run([git, "clone", "--progress", REPOSITORY_URL, directory], cwd=directory.parent)

        self._check_cancelled()
        if not venv_python(directory).is_file():
            self.log("Создание виртуального окружения…")
            self._run(python + ["-m", "venv", directory / "venv"], cwd=directory)

        if not (directory / "launch.py").is_file():
            raise InstallationError("Файл launch.py не найден в репозитории WebUI.")
        self.log("Установка зависимостей WebUI. Это может занять несколько минут…")
        self._run(
            [venv_python(directory), "-u", "launch.py", "--exit", "--skip-torch-cuda-test"],
            cwd=directory,
        )
        self._check_cancelled()
        (directory / MARKER_NAME).write_text(
            json.dumps({"name": PACKAGE_NAME, "repository": REPOSITORY_URL}, ensure_ascii=False),
            encoding="utf-8",
        )
        self.log("AUTOMATIC1111 WebUI установлен.")
        return directory


def launch_webui():
    if not is_installed():
        raise InstallationError("Пакет ещё не установлен.")
    directory = package_path()
    if os.name == "nt":
        script = directory / "webui-user.bat"
        command = ["cmd", "/c", str(script)]
        flags = subprocess.CREATE_NEW_CONSOLE
    else:
        script = directory / "webui.sh"
        command = ["bash", str(script)]
        flags = 0
    if not script.is_file():
        raise InstallationError(f"Файл запуска не найден: {script}")
    subprocess.Popen(command, cwd=directory, creationflags=flags)
