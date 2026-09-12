from socialguard_models.transcription import process_transcription


def test_orchestration_worker_receives_runtime_configuration() -> None:
    raw_function = next(
        value
        for name, value in vars(process_transcription).items()
        if name.startswith("_sync_original_")
    )

    assert [repr(secret) for secret in raw_function._spec.secrets] == [  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        "modal.Secret.from_name('socialguard-transcription-runtime')"
    ]
