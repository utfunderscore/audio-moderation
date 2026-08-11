from pathlib import Path

from torch import Tensor

class AudioMetaData:
    sample_rate: int
    num_frames: int

def info(uri: str | Path) -> AudioMetaData: ...
def load(
    uri: str | Path, *, normalize: bool, num_frames: int = ...
) -> tuple[Tensor, int]: ...

class _Functional:
    def resample(self, waveform: Tensor, orig_freq: int, new_freq: int) -> Tensor: ...

functional: _Functional
