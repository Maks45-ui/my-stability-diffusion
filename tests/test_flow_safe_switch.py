"""Behavioral check for Flow's WebUI extension without importing PyTorch."""

import os
import runpy
import sys
import threading
import types
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "src-tauri/resources/flow_safe_switch.py"


def fake_safetensors():
    package = types.ModuleType("safetensors")
    package.__path__ = []
    torch_module = types.ModuleType("safetensors.torch")
    calls = []

    def load_file(filename, device="cpu", *, backend="mmap"):
        calls.append((filename, device, backend))
        return {}

    torch_module.load_file = load_file
    package.torch = torch_module
    return package, torch_module, calls


class SafeSwitchTests(unittest.TestCase):
    def test_old_model_is_evicted_before_loading_the_new_one(self):
        calls = []
        current = types.SimpleNamespace(sd_checkpoint_info=types.SimpleNamespace(filename="sd15.safetensors"))
        target = types.SimpleNamespace(filename="sdxl.safetensors")
        data = types.SimpleNamespace(sd_model=current, loaded_sd_models=[current], lock=threading.Lock())

        def original_reload(sd_model=None, info=None, forced_reload=False):
            self.assertIsNone(data.sd_model)
            self.assertEqual(data.loaded_sd_models, [])
            calls.append("reload")
            return "loaded"

        modules = types.ModuleType("modules")
        modules.__path__ = []
        modules.sd_models = types.SimpleNamespace(
            reload_model_weights=original_reload,
            model_data=data,
            select_checkpoint=lambda: target,
            send_model_to_cpu=lambda model: calls.append("cpu"),
            send_model_to_trash=lambda model: calls.append("trash"),
        )
        modules.devices = types.SimpleNamespace(torch_gc=lambda: calls.append("gc"))
        modules.sd_hijack = types.SimpleNamespace(model_hijack=types.SimpleNamespace(undo_hijack=lambda model: calls.append("unhijack")))
        modules.sd_unet = types.SimpleNamespace(apply_unet=lambda name: calls.append("unet"))
        modules.sd_vae = types.SimpleNamespace(base_vae=object(), loaded_vae_file="old", checkpoint_info=object())
        modules.shared = types.SimpleNamespace(weight_load_location=None)
        safe_package, safe_torch, safe_calls = fake_safetensors()

        with patch.dict(os.environ, {"FLOW_SAFE_MODEL_SWITCH": "1"}), patch.dict(sys.modules, {"modules": modules, "safetensors": safe_package, "safetensors.torch": safe_torch}):
            runpy.run_path(str(SCRIPT))
            wrapped = modules.sd_models.reload_model_weights
            self.assertEqual(wrapped(), "loaded")
            self.assertEqual(calls, ["unet", "cpu", "unhijack", "trash", "gc", "reload"])
            self.assertIsNone(modules.sd_vae.base_vae)
            self.assertEqual(modules.shared.weight_load_location, "cpu")
            if os.name == "nt":
                safe_torch.load_file("checkpoint.safetensors", device="cpu")
                self.assertEqual(safe_calls, [("checkpoint.safetensors", "cpu", "pread")])

            # UI script reloads must not stack multiple wrappers.
            runpy.run_path(str(SCRIPT))
            self.assertIs(modules.sd_models.reload_model_weights, wrapped)

    def test_switch_waits_for_startup_model_load(self):
        started = threading.Event()
        release = threading.Event()
        finished = threading.Event()
        calls = []
        data = types.SimpleNamespace(sd_model=None, loaded_sd_models=[], lock=threading.Lock())

        def initial_load():
            with data.lock:
                started.set()
                if not release.wait(2):
                    raise TimeoutError("initial model load was not released")
                data.sd_model = types.SimpleNamespace(
                    sd_checkpoint_info=types.SimpleNamespace(filename="sd15.safetensors")
                )
                data.loaded_sd_models.append(data.sd_model)
                calls.append("initial load complete")

        def original_reload(sd_model=None, info=None, forced_reload=False):
            calls.append("reload")
            return "loaded"

        modules = types.ModuleType("modules")
        modules.__path__ = []
        modules.sd_models = types.SimpleNamespace(
            reload_model_weights=original_reload,
            model_data=data,
            select_checkpoint=lambda: types.SimpleNamespace(filename="sdxl.safetensors"),
            send_model_to_cpu=lambda model: None,
            send_model_to_trash=lambda model: None,
        )
        modules.devices = types.SimpleNamespace(torch_gc=lambda: None)
        modules.sd_hijack = types.SimpleNamespace(model_hijack=types.SimpleNamespace(undo_hijack=lambda model: None))
        modules.sd_unet = types.SimpleNamespace(apply_unet=lambda name: None)
        modules.sd_vae = types.SimpleNamespace(base_vae=None, loaded_vae_file=None, checkpoint_info=None)
        modules.shared = types.SimpleNamespace(weight_load_location=None)
        safe_package, safe_torch, _ = fake_safetensors()

        with patch.dict(os.environ, {"FLOW_SAFE_MODEL_SWITCH": "1"}), patch.dict(sys.modules, {"modules": modules, "safetensors": safe_package, "safetensors.torch": safe_torch}):
            runpy.run_path(str(SCRIPT))
            loader = threading.Thread(target=initial_load)
            loader.start()
            self.assertTrue(started.wait(1))

            def switch():
                modules.sd_models.reload_model_weights()
                finished.set()

            switcher = threading.Thread(target=switch)
            switcher.start()
            try:
                self.assertFalse(finished.wait(0.1))
                self.assertEqual(calls, [])
            finally:
                release.set()
                loader.join(2)
                switcher.join(2)
            self.assertFalse(loader.is_alive())
            self.assertFalse(switcher.is_alive())
            self.assertTrue(finished.is_set())
            self.assertEqual(calls, ["initial load complete", "reload"])


if __name__ == "__main__":
    unittest.main()
