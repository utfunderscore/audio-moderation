import importlib.util
import json
from functools import cache
from pathlib import Path
from types import ModuleType, SimpleNamespace
from unittest.mock import Mock

import pytest


@cache
def load_script() -> ModuleType:
    spec = importlib.util.spec_from_file_location("moderate_script", Path(__file__).parents[1] / "scripts/moderate.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_local_file_limit_is_checked_before_read(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    script = load_script()
    audio = tmp_path / "audio.wav"
    audio.write_bytes(b"12345")
    monkeypatch.setattr(script, "MAX_AUDIO_BYTES", 4)
    monkeypatch.setattr(Path, "read_bytes", lambda self: pytest.fail("oversized input must not be read"))
    with pytest.raises(ValueError, match="512 MiB"):
        script.main.info.raw_f(audio_file=str(audio))


@pytest.mark.parametrize("kwargs", [{}, {"audio_file": "audio.wav", "audio_url": "https://example.com/audio"}])
def test_exactly_one_source_is_required(kwargs: dict[str, str]) -> None:
    with pytest.raises(SystemExit, match="exactly one"):
        load_script().main.info.raw_f(**kwargs)


@pytest.mark.parametrize("local", [True, False])
def test_script_prints_json_and_timing(monkeypatch: pytest.MonkeyPatch, tmp_path: Path, capsys: pytest.CaptureFixture[str], local: bool) -> None:
    script = load_script()
    scores = {"sexual": 0.1, "hate_or_discrimination": 0.2, "harassment_or_abuse": 0.3, "violence_or_threats": 0.4, "asking_for_pii": 0.5}
    remote = Mock(return_value=scores)
    worker = SimpleNamespace(moderate=SimpleNamespace(remote=remote), moderate_bytes=SimpleNamespace(remote=remote))
    monkeypatch.setattr(script, "RobloxVoiceSafety", lambda: worker)
    if local:
        audio = tmp_path / "audio.wav"
        audio.write_bytes(b"audio")
        script.main.info.raw_f(audio_file=str(audio))
        remote.assert_called_once_with(b"audio")
    else:
        script.main.info.raw_f(audio_url="https://example.com/audio")
        remote.assert_called_once_with("https://example.com/audio", "")
    captured = capsys.readouterr()
    assert json.loads(captured.out) == scores
    assert "moderated in" in captured.err
