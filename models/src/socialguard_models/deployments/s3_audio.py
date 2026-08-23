"""Bounded S3 audio retrieval for Modal transcription workers."""

import asyncio
import os
from dataclasses import dataclass, field
from typing import Protocol, TypedDict, cast
from urllib.parse import unquote, urlsplit

import boto3
from botocore.config import Config
from botocore.exceptions import BotoCoreError, ClientError

from socialguard_models.api.networking import MAX_REMOTE_BYTES

AUDIO_TOO_LARGE_MESSAGE = "Audio exceeds the 25 MiB size limit."
AUDIO_EMPTY_MESSAGE = "Audio is empty or invalid."
DOWNLOAD_TIMEOUT_SECONDS = 90
_STREAM_CHUNK_BYTES = 64 * 1024


class AudioRetrievalError(RuntimeError):
    """The configured S3 object could not be fetched."""


class AudioRejectedError(ValueError):
    """The fetched object exceeds a public audio safety limit."""


class S3ConfigurationError(RuntimeError):
    """Required S3 client configuration is missing from the environment."""


@dataclass(frozen=True, slots=True)
class S3AudioSettings:
    """Environment-backed S3 client and bucket configuration."""

    bucket: str
    region_name: str
    access_key_id: str = field(repr=False)
    secret_access_key: str = field(repr=False)
    session_token: str | None = field(default=None, repr=False)
    endpoint_url: str | None = None

    @classmethod
    def from_environment(cls) -> "S3AudioSettings":
        """Load the S3 client parameters required by the Modal worker."""

        def required(name: str) -> str:
            value = os.environ.get(name)
            if not value:
                message = f"Missing required environment variable: {name}"
                raise S3ConfigurationError(message)
            return value

        return cls(
            bucket=required("SOCIALGUARD_S3_BUCKET"),
            region_name=required("AWS_DEFAULT_REGION"),
            access_key_id=required("AWS_ACCESS_KEY_ID"),
            secret_access_key=required("AWS_SECRET_ACCESS_KEY"),
            session_token=os.environ.get("AWS_SESSION_TOKEN") or None,
            endpoint_url=os.environ.get("SOCIALGUARD_S3_ENDPOINT_URL") or None,
        )


class _StreamingBody(Protocol):
    def read(self, amount: int) -> bytes: ...

    def close(self) -> None: ...


class _GetObjectResponse(TypedDict):
    Body: _StreamingBody
    ContentLength: int


class _S3Client(Protocol):
    def get_object(self, **kwargs: str) -> _GetObjectResponse: ...


def _create_s3_client(settings: S3AudioSettings) -> _S3Client:
    client = boto3.client(  # pyright: ignore[reportUnknownMemberType]
        "s3",
        region_name=settings.region_name,
        endpoint_url=settings.endpoint_url,
        aws_access_key_id=settings.access_key_id,
        aws_secret_access_key=settings.secret_access_key,
        aws_session_token=settings.session_token,
        config=Config(
            connect_timeout=10,
            read_timeout=30,
            retries={"max_attempts": 2, "mode": "standard"},
        ),
    )
    return cast("_S3Client", client)


def _object_key(audio_url: str, settings: S3AudioSettings) -> str:
    parsed = urlsplit(audio_url)
    if parsed.scheme != "https" or not parsed.netloc:
        message = "Audio location must be an HTTPS S3 object URL."
        raise ValueError(message)
    key = unquote(parsed.path.lstrip("/"))
    if not key:
        message = "Audio location does not contain an S3 object key."
        raise ValueError(message)
    hostname = parsed.hostname or ""
    if hostname.startswith(f"{settings.bucket}."):
        return key
    bucket_prefix = f"{settings.bucket}/"
    if key.startswith(bucket_prefix):
        return key.removeprefix(bucket_prefix)
    message = "Audio location does not reference the configured S3 bucket."
    raise ValueError(message)


def _download_audio_from_s3(
    audio_url: str,
    settings: S3AudioSettings,
    client: _S3Client | None = None,
) -> bytes:
    """Read one S3 object without buffering more than the public size limit."""
    try:
        s3 = client or _create_s3_client(settings)
        response = s3.get_object(
            Bucket=settings.bucket,
            Key=_object_key(audio_url, settings),
            Range=f"bytes=0-{MAX_REMOTE_BYTES}",
        )
        body = response["Body"]
        try:
            if response["ContentLength"] > MAX_REMOTE_BYTES:
                raise AudioRejectedError(AUDIO_TOO_LARGE_MESSAGE)
            chunks: list[bytes] = []
            size = 0
            while size <= MAX_REMOTE_BYTES:
                chunk = body.read(min(_STREAM_CHUNK_BYTES, MAX_REMOTE_BYTES + 1 - size))
                if not chunk:
                    break
                chunks.append(chunk)
                size += len(chunk)
            if size > MAX_REMOTE_BYTES:
                raise AudioRejectedError(AUDIO_TOO_LARGE_MESSAGE)
        finally:
            body.close()
    except AudioRejectedError:
        raise
    except (
        BotoCoreError,
        ClientError,
        KeyError,
        OSError,
        TypeError,
        ValueError,
    ) as exc:
        raise AudioRetrievalError from exc
    if not chunks:
        raise AudioRejectedError(AUDIO_EMPTY_MESSAGE)
    return b"".join(chunks)


async def download_audio(audio_url: str) -> bytes:
    """Download one configured S3 object without blocking the async worker."""
    try:
        settings = S3AudioSettings.from_environment()
        async with asyncio.timeout(DOWNLOAD_TIMEOUT_SECONDS):
            return await asyncio.to_thread(_download_audio_from_s3, audio_url, settings)
    except (AudioRejectedError, AudioRetrievalError):
        raise
    except (S3ConfigurationError, TimeoutError) as exc:
        raise AudioRetrievalError from exc
