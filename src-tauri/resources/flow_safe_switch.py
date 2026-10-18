"""Flow's optional low-memory checkpoint switch for AUTOMATIC1111.

Loaded as a WebUI extension. The launcher enables it only for Flow's SDXL
memory profile (or an automatically detected 4–6 GB NVIDIA GPU).
"""

import gc
import inspect
import os
from functools import wraps


def _windows_commit_status():
    """Return available/total system commit in GiB, when Windows reports it."""
    if os.name != "nt":
        return None

    import ctypes

    class MemoryStatus(ctypes.Structure):
        _fields_ = [
            ("length", ctypes.c_ulong),
            ("memory_load", ctypes.c_ulong),
            ("total_physical", ctypes.c_ulonglong),
            ("available_physical", ctypes.c_ulonglong),
            ("total_page_file", ctypes.c_ulonglong),
            ("available_page_file", ctypes.c_ulonglong),
            ("total_virtual", ctypes.c_ulonglong),
            ("available_virtual", ctypes.c_ulonglong),
            ("available_extended_virtual", ctypes.c_ulonglong),
        ]

    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        return None
    gib = 1024 ** 3
    return status.available_page_file / gib, status.total_page_file / gib


if os.environ.get("FLOW_SAFE_MODEL_SWITCH") == "1":
    import safetensors.torch
    from modules import devices, sd_hijack, sd_models, sd_unet, sd_vae, shared

    if os.name == "nt":
        original_load_file = safetensors.torch.load_file
        if not getattr(original_load_file, "_flow_pread", False):
            if "backend" not in inspect.signature(original_load_file).parameters:
                raise RuntimeError("Flow requires safetensors 0.8.0 or newer for streaming SDXL loading.")

            @wraps(original_load_file)
            def flow_load_file(filename, device="cpu", **kwargs):
                # Unlike A1111's disable-mmap setting, pread does not first
                # allocate a single byte buffer for the entire checkpoint.
                kwargs["backend"] = "pread"
                commit = _windows_commit_status()
                if commit is not None:
                    print(f"[Flow] Windows available commit before checkpoint load: {commit[0]:.1f} / {commit[1]:.1f} GiB.", flush=True)
                try:
                    return original_load_file(filename, device=device, **kwargs)
                except OSError as error:
                    if getattr(error, "winerror", None) == 1455 or "os error 1455" in str(error).lower():
                        commit = _windows_commit_status()
                        if commit is not None:
                            print(f"[Flow] Windows available commit after error 1455: {commit[0]:.1f} / {commit[1]:.1f} GiB.", flush=True)
                        print("[Flow] The Windows system commit limit was reached while loading this checkpoint. Check the Memory > Committed value in Task Manager and increase the Windows page file if needed.", flush=True)
                    raise

            flow_load_file._flow_pread = True
            safetensors.torch.load_file = flow_load_file
            print("[Flow] safetensors uses streaming pread instead of mmap.", flush=True)

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
                    commit = _windows_commit_status()
                    if commit is not None:
                        print(f"[Flow] Windows available commit before unloading: {commit[0]:.1f} / {commit[1]:.1f} GiB.", flush=True)

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
                    commit = _windows_commit_status()
                    if commit is not None:
                        print(f"[Flow] Windows available commit after unloading: {commit[0]:.1f} / {commit[1]:.1f} GiB.", flush=True)

                return original_reload(sd_model, info, forced_reload)

        flow_reload_model_weights._flow_safe_switch = True
        sd_models.reload_model_weights = flow_reload_model_weights
