"""Flow's optional low-memory checkpoint switch for AUTOMATIC1111.

Loaded as a WebUI extension. The launcher enables it only for Flow's SDXL
memory profile (or an automatically detected 4–6 GB NVIDIA GPU).
"""

import gc
import os
from functools import wraps


if os.environ.get("FLOW_SAFE_MODEL_SWITCH") == "1":
    from modules import devices, sd_hijack, sd_models, sd_unet, sd_vae, shared

    # A1111 otherwise reads .safetensors directly onto the selected device.
    # On a 4 GB GPU, stage checkpoint tensors in system RAM first and let
    # A1111's lowvram path move only the required parts to CUDA.
    shared.weight_load_location = "cpu"

    original_reload = sd_models.reload_model_weights
    if not getattr(original_reload, "_flow_safe_switch", False):

        @wraps(original_reload)
        def flow_reload_model_weights(sd_model=None, info=None, forced_reload=False):
            # AUTOMATIC1111 holds this lock while its startup thread loads the
            # first model. A settings request can otherwise enter reload_model_weights
            # at the same time and race inside safetensors/PyTorch native code.
            if sd_models.model_data.lock.locked():
                print("[Flow] Waiting for the current checkpoint load to finish...", flush=True)
            with sd_models.model_data.lock:
                current = sd_model or sd_models.model_data.sd_model
                target = info or sd_models.select_checkpoint()
                if current is not None and current.sd_checkpoint_info.filename != target.filename:
                    print("[Flow] Unloading previous checkpoint before loading the next one...", flush=True)

                    # Clear live references first: empty_cache alone cannot release
                    # tensors that are still owned by the old checkpoint.
                    sd_unet.apply_unet("None")
                    sd_models.send_model_to_cpu(current)
                    sd_hijack.model_hijack.undo_hijack(current)
                    loaded = list(sd_models.model_data.loaded_sd_models)
                    if not any(model is current for model in loaded):
                        loaded.append(current)
                    for model in loaded:
                        sd_models.send_model_to_trash(model)
                    sd_models.model_data.loaded_sd_models.clear()
                    sd_models.model_data.sd_model = None
                    sd_vae.base_vae = None
                    sd_vae.loaded_vae_file = None
                    sd_vae.checkpoint_info = None
                    loaded.clear()
                    del model
                    del current
                    sd_model = None
                    gc.collect()
                    devices.torch_gc()  # CUDA: torch.cuda.empty_cache() + ipc_collect().
                    print("[Flow] CUDA cache cleared; loading the next checkpoint.", flush=True)

                return original_reload(sd_model, info, forced_reload)

        flow_reload_model_weights._flow_safe_switch = True
        sd_models.reload_model_weights = flow_reload_model_weights
