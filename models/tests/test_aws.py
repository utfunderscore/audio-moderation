from unittest.mock import Mock, patch

import boto3
import pytest
from botocore.stub import Stubber

from socialguard_models.aws import assume_modal_oidc_role, create_presigned_download_url


def test_modal_oidc_credentials_create_authenticated_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("AWS_ROLE_ARN", " arn:aws:iam::123456789012:role/transcription ")
    monkeypatch.setenv("MODAL_IDENTITY_TOKEN", " identity-token ")
    sts = boto3.client(
        "sts",
        region_name="us-east-1",
        aws_access_key_id="testing",
        aws_secret_access_key="testing",
    )
    with Stubber(sts) as stubber:
        stubber.add_response(
            "assume_role_with_web_identity",
            {
                "Credentials": {
                    "AccessKeyId": "ASIAEXAMPLEKEY123",
                    "SecretAccessKey": "temporary-secret",
                    "SessionToken": "temporary-token",
                    "Expiration": "2030-01-01T00:00:00Z",
                }
            },
            {
                "RoleArn": "arn:aws:iam::123456789012:role/transcription",
                "RoleSessionName": "modal-session",
                "WebIdentityToken": "identity-token",
            },
        )
        with patch("socialguard_models.aws.boto3.Session.client", return_value=sts):
            session = assume_modal_oidc_role()
        stubber.assert_no_pending_responses()

    credentials = session.get_credentials().get_frozen_credentials()
    assert credentials.access_key == "ASIAEXAMPLEKEY123"
    assert credentials.secret_key == "temporary-secret"
    assert credentials.token == "temporary-token"


@pytest.mark.parametrize("missing_variable", ["AWS_ROLE_ARN", "MODAL_IDENTITY_TOKEN"])
def test_modal_oidc_requires_environment_credentials(
    monkeypatch: pytest.MonkeyPatch, missing_variable: str,
) -> None:
    monkeypatch.setenv("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/transcription")
    monkeypatch.setenv("MODAL_IDENTITY_TOKEN", "identity-token")
    monkeypatch.setenv(missing_variable, " ")

    with pytest.raises(RuntimeError, match=missing_variable):
        assume_modal_oidc_role()


def test_presigned_download_url_uses_configured_s3_region(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("AWS_REGION", "eu-west-2")
    s3 = Mock()
    s3.generate_presigned_url.return_value = "https://example.com/audio.wav"
    session = Mock()
    session.client.return_value = s3

    result = create_presigned_download_url(session, "s3://audio-bucket/path/audio.wav")

    assert result == "https://example.com/audio.wav"
    session.client.assert_called_once_with("s3", region_name="eu-west-2")
    s3.generate_presigned_url.assert_called_once_with(
        "get_object",
        Params={"Bucket": "audio-bucket", "Key": "path/audio.wav"},
        ExpiresIn=3_600,
    )


def test_presigned_download_url_requires_s3_region(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("AWS_REGION", raising=False)

    with pytest.raises(RuntimeError, match="AWS_REGION"):
        create_presigned_download_url(Mock(), "s3://audio-bucket/path/audio.wav")
