"""Optional Discord activity for Flow."""

import logging
from threading import Condition, Thread
from time import monotonic, time

from pypresence import Presence


CLIENT_ID = "1559517880825942036"
PAGE_STATES = {
    "Главная": "На главном экране",
    "Галерея": "Просмотр галереи",
    "Загрузка моделей": "Загрузка моделей",
    "Настройки": "Настройки",
}


class DiscordPresence:
    UPDATE_INTERVAL = 15.0

    def __init__(self):
        self.started_at = int(time() * 1000)
        self._state = PAGE_STATES["Главная"]
        self._closed = False
        self._condition = Condition()
        Thread(target=self._run, name="Flow Discord Presence", daemon=True).start()

    def set_page(self, page_name):
        with self._condition:
            self._state = PAGE_STATES[page_name]
            self._condition.notify()

    def close(self):
        with self._condition:
            self._closed = True
            self._condition.notify()

    def _run(self):
        rpc = None
        try:
            rpc = Presence(CLIENT_ID)
            rpc.connect()
            last_state = None
            next_update_at = 0.0

            while True:
                with self._condition:
                    while not self._closed:
                        state = self._state
                        wait_seconds = max(0.0, next_update_at - monotonic())
                        if state != last_state and wait_seconds == 0:
                            break
                        self._condition.wait(
                            wait_seconds if state != last_state else None
                        )
                    if self._closed:
                        return

                rpc.update(name="Flow", state=state, start=self.started_at)
                last_state = state
                next_update_at = monotonic() + self.UPDATE_INTERVAL
        except Exception:
            logging.getLogger(__name__).debug(
                "Discord Rich Presence unavailable", exc_info=True
            )
        finally:
            if rpc is not None:
                try:
                    rpc.close()
                except Exception:
                    pass
