"""Shared immutable resources for the Granite Modal integration spike."""

import modal

MODEL_ID = "ibm-granite/granite-4.0-1b-speech"
MODEL_REVISION = "bd87ab862416353633ea431fe49b1614003623c5"
CACHE_DIRECTORY = "/cache/huggingface"
VOLUME_NAME = "socialguard-granite-4-0-1b-speech-hf-cache"
GPU = "L40S"
CPU_CORES = 2.0
HOST_MEMORY_MIB = 16_384
MAX_ACTIVE_INPUTS = 1
VENDOR_MAX_NEW_TOKENS = 200

MODEL_CACHE = modal.Volume.from_name(VOLUME_NAME, create_if_missing=True)
IMAGE = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("ffmpeg", "libsndfile1")
    .uv_sync(
        uv_project_dir=".",
        groups=["backend-granite"],
        frozen=True,
        extra_options="--no-dev",
    )
    .env({"HF_HUB_CACHE": CACHE_DIRECTORY})
    .add_local_python_source("socialguard_models")
)
