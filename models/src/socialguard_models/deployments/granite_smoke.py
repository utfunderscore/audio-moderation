"""GPU-only Modal smoke test for the pinned Granite Speech model."""

from pathlib import Path

import modal

from socialguard_models.deployments.granite_resources import (
    CACHE_DIRECTORY,
    CPU_CORES,
    GPU,
    HOST_MEMORY_MIB,
    IMAGE,
    MAX_ACTIVE_INPUTS,
    MODEL_CACHE,
    MODEL_ID,
    MODEL_REVISION,
    VENDOR_MAX_NEW_TOKENS,
)

EXPECTED_SAMPLE_RATE = 16_000
USER_PROMPT = "<|audio|>can you transcribe the speech into a written format?"
SMOKE_APP = modal.App("socialguard-granite-smoke")


@SMOKE_APP.function(  # pyright: ignore[reportUnknownMemberType]
    image=IMAGE,
    gpu=GPU,
    cpu=CPU_CORES,
    memory=HOST_MEMORY_MIB,
    volumes={CACHE_DIRECTORY: MODEL_CACHE.with_mount_options(read_only=True)},
    timeout=1_200,
)
@modal.concurrent(  # pyright: ignore[reportUnknownMemberType]
    max_inputs=MAX_ACTIVE_INPUTS,
)
def smoke_transcription() -> str:
    """Transcribe IBM's bundled sample with one L40S GPU."""
    import torch
    import torchaudio
    from huggingface_hub import snapshot_download
    from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

    snapshot_directory = Path(
        snapshot_download(
            repo_id=MODEL_ID,
            revision=MODEL_REVISION,
            cache_dir=CACHE_DIRECTORY,
            local_files_only=True,
        ),
    )

    waveform, sample_rate = torchaudio.load(
        snapshot_directory / "multilingual_sample.wav",
        normalize=True,
    )
    if waveform.shape[0] != 1 or sample_rate != EXPECTED_SAMPLE_RATE:
        message = "The bundled Granite smoke fixture must be mono 16 kHz audio."
        raise ValueError(message)

    processor = AutoProcessor.from_pretrained(snapshot_directory)
    model = (
        AutoModelForSpeechSeq2Seq.from_pretrained(
            snapshot_directory,
            torch_dtype=torch.bfloat16,
        )
        .eval()
        .to("cuda")
    )

    prompt = processor.tokenizer.apply_chat_template(
        [{"role": "user", "content": USER_PROMPT}],
        tokenize=False,
        add_generation_prompt=True,
    )
    model_inputs = processor(
        prompt,
        waveform,
        device="cuda",
        return_tensors="pt",
    ).to("cuda")
    with torch.inference_mode():
        model_outputs = model.generate(
            **model_inputs,
            max_new_tokens=VENDOR_MAX_NEW_TOKENS,
            do_sample=False,
            num_beams=1,
        )

    num_input_tokens = model_inputs["input_ids"].shape[-1]
    new_tokens = model_outputs[0, num_input_tokens:].unsqueeze(0)
    transcription = processor.tokenizer.batch_decode(
        new_tokens,
        add_special_tokens=False,
        skip_special_tokens=True,
    )
    if len(transcription) != 1:
        message = "The Granite smoke test must return exactly one transcript."
        raise RuntimeError(message)
    return transcription[0]
