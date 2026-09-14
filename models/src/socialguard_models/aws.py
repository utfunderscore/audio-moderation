"""Utilities for authenticating with AWS."""

import os
from typing import Protocol, TypedDict, cast
from urllib.parse import unquote, urlsplit

import boto3


class _Credentials(TypedDict):
    AccessKeyId: str
    SecretAccessKey: str
    SessionToken: str


class _AssumeRoleResponse(TypedDict):
    Credentials: _Credentials


class _StsClient(Protocol):
    def assume_role_with_web_identity(
        self,
        *,
        RoleArn: str,
        RoleSessionName: str,
        WebIdentityToken: str,
    ) -> _AssumeRoleResponse: ...


class _S3Client(Protocol):
    def generate_presigned_url(
        self,
        ClientMethod: str,
        Params: dict[str, str],
        ExpiresIn: int,
    ) -> str: ...


def create_presigned_download_url(
    session: boto3.Session,
    audio_uri: str,
    *,
    expires_in: int = 3_600,
) -> str:
    """Create a presigned GET URL for an object identified by an S3 URI."""
    parsed = urlsplit(audio_uri)
    if parsed.scheme != "s3" or not parsed.netloc or len(parsed.path) <= 1:
        raise ValueError("audio_uri must use the format s3://bucket/object-key")
    if parsed.query or parsed.fragment:
        raise ValueError("audio_uri must not contain a query string or fragment")
    if not 1 <= expires_in <= 604_800:
        raise ValueError("expires_in must be between 1 and 604800")
    region = os.environ.get("AWS_REGION", "").strip()
    if not region:
        raise RuntimeError("AWS_REGION is not configured")

    s3 = cast(
        _S3Client,
        session.client("s3", region_name=region),  # pyright: ignore[reportUnknownMemberType]
    )
    return s3.generate_presigned_url(
        "get_object",
        Params={"Bucket": parsed.netloc, "Key": unquote(parsed.path[1:])},
        ExpiresIn=expires_in,
    )


def assume_modal_oidc_role() -> boto3.Session:
    """Assume the AWS role configured for the current Modal Function."""
    role_arn = os.environ.get("AWS_ROLE_ARN", "").strip()
    if not role_arn:
        raise RuntimeError("AWS_ROLE_ARN is not configured")

    identity_token = os.environ.get("MODAL_IDENTITY_TOKEN", "").strip()
    if not identity_token:
        raise RuntimeError("MODAL_IDENTITY_TOKEN is not available")

    sts = cast(
        _StsClient,
        boto3.Session().client("sts"),  # pyright: ignore[reportUnknownMemberType]
    )
    credentials = sts.assume_role_with_web_identity(
        RoleArn=role_arn,
        RoleSessionName="modal-session",
        WebIdentityToken=identity_token,
    )["Credentials"]
    return boto3.Session(
        aws_access_key_id=credentials["AccessKeyId"],
        aws_secret_access_key=credentials["SecretAccessKey"],
        aws_session_token=credentials["SessionToken"],
    )
