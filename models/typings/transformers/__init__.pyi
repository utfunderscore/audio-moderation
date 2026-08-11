from collections.abc import Mapping
from pathlib import Path

from torch import Tensor

class Tokenizer:
    def apply_chat_template(
        self,
        conversation: list[dict[str, str]],
        *,
        tokenize: bool,
        add_generation_prompt: bool,
    ) -> str: ...
    def batch_decode(
        self, sequences: Tensor, *, add_special_tokens: bool, skip_special_tokens: bool
    ) -> list[str]: ...

class ModelInputs(Mapping[str, Tensor]):
    def to(self, device: str) -> ModelInputs: ...

class Processor:
    tokenizer: Tokenizer
    def __call__(
        self, prompt: str, waveform: Tensor, *, device: str, return_tensors: str
    ) -> ModelInputs: ...

class SpeechModel:
    def eval(self) -> SpeechModel: ...
    def to(self, device: str) -> SpeechModel: ...
    def generate(
        self, *, max_new_tokens: int, do_sample: bool, num_beams: int, **inputs: Tensor
    ) -> Tensor: ...

class AutoProcessor:
    @classmethod
    def from_pretrained(
        cls, path: Path, *, local_files_only: bool = ...
    ) -> Processor: ...

class AutoModelForSpeechSeq2Seq:
    @classmethod
    def from_pretrained(
        cls, path: Path, *, local_files_only: bool = ..., torch_dtype: object
    ) -> SpeechModel: ...
