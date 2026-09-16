import pytest

from socialguard_models.moderation.models.roblox_voice_safety_audio import (
    ALIGNMENT_SAMPLES,
    MIN_INFERENCE_SAMPLES,
    STRIDE_SAMPLES,
    WINDOW_SAMPLES,
    audio_windows,
    pad_pcm_window,
)


@pytest.mark.parametrize("samples", [1, 200, 1_279, 16_001, 239_999, 240_000, 240_001, 480_000, 480_123, 4_800_000])
def test_windows_cover_every_sample_without_exceeding_model_limit(samples: int) -> None:
    windows = list(audio_windows(samples))
    assert windows[0][0] == 0
    assert windows[-1][1] == samples
    assert len(windows) == len(set(windows))
    for start, end in windows:
        assert 0 <= start < end <= samples
        assert end - start <= WINDOW_SAMPLES
    for previous, following in zip(windows, windows[1:]):
        assert previous[0] < following[0]
        assert following[0] - previous[0] <= STRIDE_SAMPLES
        assert following[0] < previous[1]
    if samples > WINDOW_SAMPLES:
        assert all(end - start == WINDOW_SAMPLES for start, end in windows)


def test_five_minutes_uses_25_windows_and_aligned_end_is_not_duplicated() -> None:
    assert len(list(audio_windows(4_800_000))) == 25
    assert list(audio_windows(WINDOW_SAMPLES + STRIDE_SAMPLES)) == [
        (0, WINDOW_SAMPLES), (STRIDE_SAMPLES, STRIDE_SAMPLES + WINDOW_SAMPLES),
    ]


@pytest.mark.parametrize("samples", [0, -1])
def test_empty_audio_has_no_successful_window(samples: int) -> None:
    with pytest.raises(ValueError):
        list(audio_windows(samples))


@pytest.mark.parametrize("samples", [1, 201, 1_279, 1_280, 1_281, 16_001, 239_999, 240_000])
def test_padding_retains_original_pcm_and_adds_only_bounded_silence(samples: int) -> None:
    pcm = b"\x01\x02" * samples
    padded = pad_pcm_window(pcm)
    assert padded[:len(pcm)] == pcm
    assert not any(padded[len(pcm):])
    length = len(padded) // 2
    assert MIN_INFERENCE_SAMPLES <= length <= WINDOW_SAMPLES
    assert length % ALIGNMENT_SAMPLES == 0
    assert length - samples < max(MIN_INFERENCE_SAMPLES, ALIGNMENT_SAMPLES)


@pytest.mark.parametrize("pcm", [b"", b"1", bytes(480_002)])
def test_invalid_window_is_rejected(pcm: bytes) -> None:
    with pytest.raises(ValueError):
        pad_pcm_window(pcm)
