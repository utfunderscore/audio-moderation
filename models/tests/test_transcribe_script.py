import importlib.util
from pathlib import Path
from types import ModuleType

import pytest


def _load_script() -> ModuleType:
    script_path = Path(__file__).parents[1] / "scripts" / "transcribe.py"
    spec = importlib.util.spec_from_file_location("transcribe_script", script_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_local_file_over_size_limit_is_not_read(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    transcribe_script = _load_script()
    audio_path = tmp_path / "audio.wav"
    audio_path.write_bytes(b"12345")
    monkeypatch.setattr(transcribe_script, "MAX_AUDIO_BYTES", 4)
    monkeypatch.setattr(
        Path,
        "read_bytes",
        lambda self: pytest.fail("oversized audio must not be read"),
    )

    with pytest.raises(ValueError, match="audio file exceeds the 512 MiB limit"):
        transcribe_script.main.info.raw_f(audio_file=str(audio_path))
