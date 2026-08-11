"""Generate the checked-in OpenAPI definition for the ASR gateway."""

import json
from pathlib import Path

from socialguard_models.api.app import create_gateway_app
from socialguard_models.api.auth import GatewaySettings
from socialguard_models.api.submission import SubmissionCommand, SubmissionResult

OPENAPI_ARTIFACT = Path("openapi/gateway.openapi.json")
_OPENAPI_DEFINITION_TOKEN = "openapi-definition-token"  # noqa: S105


class _DefinitionSubmitter:
    """No-op submitter used only to construct the API definition."""

    async def submit(self, command: SubmissionCommand) -> SubmissionResult:
        """Reject accidental runtime use while generating static API metadata."""
        del command
        message = "The OpenAPI definition submitter cannot accept jobs."
        raise RuntimeError(message)


def render_openapi() -> str:
    """Return deterministic JSON for the gateway's generated OpenAPI document."""
    document = create_gateway_app(
        _DefinitionSubmitter(),
        GatewaySettings(api_token=_OPENAPI_DEFINITION_TOKEN),
    ).openapi()
    return json.dumps(document, indent=2, sort_keys=True) + "\n"


def write_openapi(path: Path = OPENAPI_ARTIFACT) -> None:
    """Write the deterministic OpenAPI artifact to its repository location."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_openapi(), encoding="utf-8")


if __name__ == "__main__":
    write_openapi()
