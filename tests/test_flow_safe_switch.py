"""Behavioral check for Flow's WebUI extension without importing PyTorch."""

import os
import runpy
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "src-tauri/resources/flow_safe_switch.py"


class SafeSwitchTests(unittest.TestCase):
    def test_old_model_is_evicted_before_loading_the_new_one(self):
        calls = []
        current = types.SimpleNamespace(sd_checkpoint_info=types.SimpleNamespace(filename="sd15.safetensors"))
        target = types.SimpleNamespace(filename="sdxl.safetensors")
        data = types.SimpleNamespace(sd_model=current, loaded_sd_models=[current])

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

        with patch.dict(os.environ, {"FLOW_SAFE_MODEL_SWITCH": "1"}), patch.dict(sys.modules, {"modules": modules}):
            runpy.run_path(str(SCRIPT))
            wrapped = modules.sd_models.reload_model_weights
            self.assertEqual(wrapped(), "loaded")
            self.assertEqual(calls, ["unet", "cpu", "unhijack", "trash", "gc", "reload"])
            self.assertIsNone(modules.sd_vae.base_vae)

            # UI script reloads must not stack multiple wrappers.
            runpy.run_path(str(SCRIPT))
            self.assertIs(modules.sd_models.reload_model_weights, wrapped)


if __name__ == "__main__":
    unittest.main()
