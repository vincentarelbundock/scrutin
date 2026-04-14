import os
import warnings

import pytest

from scrutindemo_py import add, subtract, divide


def test_add_passes():
    assert add(2, 3) == 5
    assert add(-1, 1) == 0


def test_subtract_fails_due_to_bug():
    assert subtract(5, 3) == 2


def test_divide_warns_on_zero():
    with pytest.warns(RuntimeWarning, match="division by zero"):
        divide(1, 0)


def test_divide_unhandled_warning():
    warnings.simplefilter("always")
    result = divide(10, 0)
    assert result == float("inf")


@pytest.mark.skipif(
    os.environ.get("RUN_SLOW_TESTS") != "1",
    reason="set RUN_SLOW_TESTS=1 to enable",
)
def test_slow_integration():
    assert add(1, 1) == 2
