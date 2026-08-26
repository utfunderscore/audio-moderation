"""Cheap local tests for Granite Modal integration configuration."""

import inspect

import boto3
import pytest

from socialguard_models.api.callbacks import CallbackAuthenticationError
from socialguard_models.deployments.granite import (
    APP,
    CALLBACK_ENV,
    GATEWAY_MAX_CONTAINERS,
    GPU_STARTUP_TIMEOUT_SECONDS,
    WORKER_MAX_CONTAINERS,
    WORKER_TIMEOUT_SECONDS,
    _assume_callback_role,  # pyright: ignore[reportPrivateUsage]
    process_transcription,
)
from socialguard_models.deployments.granite_prefetch import PREFETCH_APP
from socialguard_models.deployments.granite_resources import (
    CPU_CORES,
    GPU,
    HOST_MEMORY_MIB,
    MAX_ACTIVE_INPUTS,
    MODEL_ID,
    MODEL_REVISION,
    VENDOR_MAX_NEW_TOKENS,
)
from socialguard_models.deployments.granite_smoke import SMOKE_APP

EXPECTED_CPU_CORES = 2.0
EXPECTED_HOST_MEMORY_MIB = 16_384
EXPECTED_MAX_NEW_TOKENS = 200
EXPECTED_MAX_CONTAINERS = 2
EXPECTED_WORKER_TIMEOUT_SECONDS = 420
EXPECTED_GPU_STARTUP_TIMEOUT_SECONDS = 600


def test_gateway_deployment_registers_bounded_cpu_and_gpu_functions() -> None:
    """The production module imports locally and registers the expected Modal parts."""
    registered_function_names = set(APP.registered_functions)  # pyright: ignore[reportUnknownMemberType,reportUnknownArgumentType]
    assert {
        "gateway",
        "process_transcription",
        "GraniteModel.*",
    } <= registered_function_names
    assert "GraniteModel" in APP.registered_classes
    assert GATEWAY_MAX_CONTAINERS == EXPECTED_MAX_CONTAINERS
    assert WORKER_MAX_CONTAINERS == EXPECTED_MAX_CONTAINERS
    assert WORKER_TIMEOUT_SECONDS == EXPECTED_WORKER_TIMEOUT_SECONDS
    assert GPU_STARTUP_TIMEOUT_SECONDS == EXPECTED_GPU_STARTUP_TIMEOUT_SECONDS


def test_worker_receives_ledger_key_to_repair_dispatch_state() -> None:
    """The spawned worker can repair the gateway's post-spawn state-marker gap."""
    parameters = inspect.signature(process_transcription.get_raw_f()).parameters

    assert tuple(parameters) == ("ledger_key", "transcription_id", "command")


def test_worker_has_development_aws_callback_configuration() -> None:
    """The worker receives only non-secret identifiers for the deployed AWS target."""
    assert CALLBACK_ENV == {
        "ASR_CALLBACK_URL": (
            "https://e4rytge57sepkksrukjcy3p7ze0ezqnl.lambda-url.eu-west-2.on.aws/"
        ),
        "ASR_CALLBACK_ROLE_ARN": (
            "arn:aws:iam::967883357915:role/socialguard-dev-modal-asr-callback-sender"
        ),
        "AWS_REGION": "eu-west-2",
        "AWS_SIGV4_SERVICE": "lambda",
    }


def test_modal_identity_is_exchanged_for_temporary_callback_credentials(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The runtime OIDC token goes only to STS and the session token reaches SigV4."""
    request: dict[str, str] = {}

    class StsClient:
        def assume_role_with_web_identity(self, **kwargs: str) -> dict[str, object]:
            request.update(kwargs)
            return {
                "Credentials": {
                    "AccessKeyId": "temporary-access-key",
                    "SecretAccessKey": "temporary-secret-key",
                    "SessionToken": "temporary-session-token",
                }
            }

    def fake_client(service_name: str, **kwargs: object) -> StsClient:
        del kwargs
        assert service_name == "sts"
        return StsClient()

    for key, value in CALLBACK_ENV.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setenv("MODAL_IDENTITY_TOKEN", "modal-oidc-token")
    monkeypatch.setattr(boto3, "client", fake_client)

    signer = _assume_callback_role()
    headers = signer.headers(CALLBACK_ENV["ASR_CALLBACK_URL"], b"{}")

    assert request["RoleArn"] == CALLBACK_ENV["ASR_CALLBACK_ROLE_ARN"]
    assert request["WebIdentityToken"] == "modal-oidc-token"
    assert request["RoleSessionName"].startswith("asr-")
    assert headers["X-Amz-Security-Token"] == "temporary-session-token"


def test_missing_modal_identity_token_is_an_authentication_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A local or misconfigured worker fails before contacting AWS STS."""
    monkeypatch.delenv("MODAL_IDENTITY_TOKEN", raising=False)
    with pytest.raises(CallbackAuthenticationError):
        _assume_callback_role()


def test_granite_resources_pin_the_model_and_gpu() -> None:
    """The integration targets one immutable Granite revision on an L40S."""
    assert MODEL_ID == "ibm-granite/granite-4.0-1b-speech"
    assert MODEL_REVISION == "bd87ab862416353633ea431fe49b1614003623c5"
    assert GPU == "L40S"


def test_prefetch_and_smoke_use_distinct_modal_apps() -> None:
    """CPU prefetch registration does not include the paid GPU smoke App."""
    assert PREFETCH_APP.name == "socialguard-granite-prefetch"
    assert SMOKE_APP.name == "socialguard-granite-smoke"
    assert PREFETCH_APP.name != SMOKE_APP.name


def test_smoke_resources_and_generation_limits_are_conservative() -> None:
    """The smoke test reserves host resources and uses one deterministic input."""
    assert CPU_CORES == EXPECTED_CPU_CORES
    assert HOST_MEMORY_MIB == EXPECTED_HOST_MEMORY_MIB
    assert MAX_ACTIVE_INPUTS == 1
    assert VENDOR_MAX_NEW_TOKENS == EXPECTED_MAX_NEW_TOKENS
