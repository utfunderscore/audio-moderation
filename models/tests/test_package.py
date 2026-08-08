"""Package-level smoke tests."""

import socialguard_models


def test_package_is_importable() -> None:
    """The installed source package can be imported."""
    assert socialguard_models.__doc__
