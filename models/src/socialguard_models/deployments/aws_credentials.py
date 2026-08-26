"""Temporary AWS credentials obtained from Modal's runtime identity."""

import os
from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Protocol, cast
from uuid import uuid4

import boto3
from botocore import UNSIGNED
from botocore.config import Config
from botocore.exceptions import BotoCoreError, ClientError


class TemporaryAwsCredentialsError(RuntimeError):
    """Modal's OIDC identity could not be exchanged for AWS credentials."""


@dataclass(frozen=True, slots=True)
class TemporaryAwsCredentials:
    """Short-lived STS session credentials kept only in worker memory."""

    access_key_id: str = field(repr=False)
    secret_access_key: str = field(repr=False)
    session_token: str = field(repr=False)


class _StsClient(Protocol):
    """Typed subset of STS used without adding the full service stub package."""

    def assume_role_with_web_identity(
        self,
        *,
        RoleArn: str,  # noqa: N803 - AWS request field spelling.
        RoleSessionName: str,  # noqa: N803 - AWS request field spelling.
        WebIdentityToken: str,  # noqa: N803 - AWS request field spelling.
    ) -> Mapping[str, object]: ...


def assume_role_with_modal_identity(
    *, role_arn: str, region: str
) -> TemporaryAwsCredentials:
    """Exchange Modal's runtime OIDC token for one short-lived AWS session."""
    identity_token = os.environ.get("MODAL_IDENTITY_TOKEN")
    if not identity_token:
        message = "MODAL_IDENTITY_TOKEN is unavailable"
        raise TemporaryAwsCredentialsError(message)

    try:
        sts = cast(
            "_StsClient",
            boto3.client(  # pyright: ignore[reportUnknownMemberType]
                "sts",
                region_name=region,
                config=Config(
                    signature_version=UNSIGNED,
                    connect_timeout=5,
                    read_timeout=10,
                    retries={"max_attempts": 2, "mode": "standard"},
                ),
            ),
        )
        response = sts.assume_role_with_web_identity(
            RoleArn=role_arn,
            RoleSessionName=f"asr-{uuid4().hex}",
            WebIdentityToken=identity_token,
        )
    except (BotoCoreError, ClientError) as exc:
        message = "AWS rejected the Modal OIDC role exchange"
        raise TemporaryAwsCredentialsError(message) from exc

    credentials_value = response.get("Credentials")
    if not isinstance(credentials_value, Mapping):
        message = "AWS STS returned no temporary credentials"
        raise TemporaryAwsCredentialsError(message)
    credentials = cast("Mapping[str, object]", credentials_value)
    values: dict[str, object] = {
        key: credentials.get(key)
        for key in ("AccessKeyId", "SecretAccessKey", "SessionToken")
    }
    if not all(isinstance(value, str) and value for value in values.values()):
        message = "AWS STS returned incomplete temporary credentials"
        raise TemporaryAwsCredentialsError(message)
    return TemporaryAwsCredentials(
        access_key_id=cast("str", values["AccessKeyId"]),
        secret_access_key=cast("str", values["SecretAccessKey"]),
        session_token=cast("str", values["SessionToken"]),
    )
