from pathlib import Path

def snapshot_download(
    repo_id: str,
    *,
    revision: str | None = None,
    cache_dir: str | Path | None = None,
    local_dir: str | Path | None = None,
    local_files_only: bool = False,
    allow_patterns: str | list[str] | None = None,
) -> str: ...
