# socialguard-models

Typed ASR model deployments for Modal.

## Requirements

- Python 3.12
- [`uv`](https://docs.astral.sh/uv/)

## Set up

```bash
uv sync
```

`uv sync` creates `.venv`, installs this package, and installs the default `dev`
dependency group.

## Validate

```bash
uv lock --check
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
```

## Dependency policy

Shared HTTP and Modal dependencies belong in `[project.dependencies]`.
Development-only tools belong in `[dependency-groups].dev`.
Each ASR backend will receive its own dependency group so heavy model frameworks are
not installed in the default development environment.
