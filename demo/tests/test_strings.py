import pytest

from scrutindemo_py import shout, fetch_remote


def test_shout_uppercases():
    assert shout("hello") == "HELLO"


def test_fetch_remote_errors_out():
    fetch_remote("https://example.invalid")


@pytest.mark.skip(reason="not implemented yet")
def test_unimplemented_feature():
    assert False, "should never run"
