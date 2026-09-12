"""Shared Modal application definition."""

import modal


app = modal.App("socialguard-transcription")

cpu_image = (
    modal.Image.debian_slim(python_version="3.12")
    .pip_install(
        "boto3>=1.35",
        "fastapi[standard]>=0.115",
        "pydantic>=2.9",
    )
    .add_local_python_source("socialguard_models")
)
