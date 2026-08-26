"""Tests for bounded, environment-configured S3 audio retrieval."""

# pyright: reportPrivateUsage=false

import pytest

from socialguard_models.api.networking import MAX_REMOTE_BYTES
from socialguard_models.deployments import s3_audio


class FakeStreamingBody:
    """Small in-memory stand-in for botocore's streaming response body."""

    def __init__(self, content: bytes) -> None:
        self.content = content
        self.closed = False

    def read(self, amount: int) -> bytes:
        chunk = self.content[:amount]
        self.content = self.content[amount:]
        return chunk

    def close(self) -> None:
        self.closed = True


class FakeS3Client:
    """Record one get-object request and return a controlled body."""

    def __init__(self, body: FakeStreamingBody, content_length: int) -> None:
        self.body = body
        self.content_length = content_length
        self.request: tuple[str, str, str] | None = None

    def get_object(self, **kwargs: str) -> s3_audio._GetObjectResponse:
        self.request = (kwargs["Bucket"], kwargs["Key"], kwargs["Range"])
        return {"Body": self.body, "ContentLength": self.content_length}


@pytest.fixture
def s3_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    """Configure every supported S3 environment variable for one test."""
    monkeypatch.setenv("AWS_DEFAULT_REGION", "eu-west-2")
    monkeypatch.setenv("AWS_ACCESS_KEY_ID", "test-access-key")
    monkeypatch.setenv("AWS_SECRET_ACCESS_KEY", "test-secret-key")
    monkeypatch.setenv("AWS_SESSION_TOKEN", "test-session-token")
    monkeypatch.setenv("SOCIALGUARD_S3_ENDPOINT_URL", "https://s3.example.com")


def test_s3_settings_load_client_parameters_without_repr_secrets(
    s3_environment: None,
) -> None:
    """The worker reads all client parameters without exposing credential values."""
    del s3_environment

    settings = s3_audio.S3AudioSettings.from_environment()

    assert settings.region_name == "eu-west-2"
    assert settings.access_key_id == "test-access-key"
    assert settings.secret_access_key == "test-secret-key"
    assert settings.session_token == "test-session-token"
    assert settings.endpoint_url == "https://s3.example.com"
    assert "test-access-key" not in repr(settings)
    assert "test-secret-key" not in repr(settings)
    assert "test-session-token" not in repr(settings)


def test_s3_settings_require_region(monkeypatch: pytest.MonkeyPatch) -> None:
    """Missing required configuration fails before boto3 can make a request."""
    monkeypatch.delenv("AWS_DEFAULT_REGION", raising=False)

    with pytest.raises(s3_audio.S3ConfigurationError, match="AWS_DEFAULT_REGION"):
        s3_audio.S3AudioSettings.from_environment()


def test_download_streams_s3_uri_bucket_key_with_bounded_range() -> None:
    """The URI bucket and decoded object key drive the bounded get-object request."""
    settings = s3_audio.S3AudioSettings(
        region_name="eu-west-2",
        access_key_id="access",
        secret_access_key="secret",
    )
    body = FakeStreamingBody(b"audio bytes")
    client = FakeS3Client(body, len(b"audio bytes"))

    audio = s3_audio._download_audio_from_s3(
        "s3://workflow-audio/segments/audio%20clip.mp3",
        settings,
        client,
    )

    assert audio == b"audio bytes"
    assert client.request == (
        "workflow-audio",
        "segments/audio clip.mp3",
        f"bytes=0-{MAX_REMOTE_BYTES}",
    )
    assert body.closed


def test_download_rejects_oversized_s3_object_and_closes_body() -> None:
    """The range response detects an oversized object before buffering it."""
    settings = s3_audio.S3AudioSettings("eu-west-2", "id", "key")
    body = FakeStreamingBody(b"not read")
    client = FakeS3Client(body, MAX_REMOTE_BYTES + 1)

    with pytest.raises(s3_audio.AudioRejectedError, match="25 MiB"):
        s3_audio._download_audio_from_s3(
            "s3://audio-bucket/large.mp3", settings, client
        )

    assert body.content == b"not read"
    assert body.closed


def test_download_uses_any_bucket_authorized_by_the_aws_identity() -> None:
    """Bucket restrictions belong to IAM rather than local configuration."""
    settings = s3_audio.S3AudioSettings("eu-west-2", "id", "key")
    client = FakeS3Client(FakeStreamingBody(b"audio"), len(b"audio"))

    assert (
        s3_audio._download_audio_from_s3(
            "s3://other-authorized-bucket/audio.mp3", settings, client
        )
        == b"audio"
    )
    assert client.request == (
        "other-authorized-bucket",
        "audio.mp3",
        f"bytes=0-{MAX_REMOTE_BYTES}",
    )


@pytest.mark.parametrize(
    "audio_uri",
    [
        "https://audio-bucket.s3.amazonaws.com/audio.mp3",
        "s3://audio-bucket",
        "s3://audio-bucket/audio.mp3?versionId=1",
    ],
)
def test_download_rejects_non_s3_object_uris(audio_uri: str) -> None:
    """Malformed locations fail before boto3 receives a request."""
    settings = s3_audio.S3AudioSettings("eu-west-2", "id", "key")
    client = FakeS3Client(FakeStreamingBody(b"audio"), len(b"audio"))

    with pytest.raises(s3_audio.AudioRetrievalError):
        s3_audio._download_audio_from_s3(audio_uri, settings, client)

    assert client.request is None
