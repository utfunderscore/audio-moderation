from unittest.mock import patch

import boto3
import pytest
from botocore.stub import Stubber

from socialguard_models.aws import assume_modal_oidc_role


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
