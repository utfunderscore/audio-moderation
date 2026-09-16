"""Shared Modal application definition."""

import modal


app = modal.App("socialguard-transcription")

runtime_secret = modal.Secret.from_name(
    "socialguard-transcription-runtime",
    required_keys=["AWS_REGION", "AWS_ROLE_ARN"],
)

cpu_image = (
    modal.Image.debian_slim(python_version="3.12")
    .pip_install(
        "boto3>=1.35",
        "fastapi[standard]>=0.115",
        "pydantic>=2.9",
    )
    .add_local_python_source("socialguard_models")
)
