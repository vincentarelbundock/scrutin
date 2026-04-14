"""Toy package for exercising scrutin's pytest plugin."""

import warnings


def add(x: int, y: int) -> int:
    return x + y


def subtract(x: int, y: int) -> int:
    """Intentionally buggy: returns x + y so subtraction tests fail."""
    return x + y


def divide(x: float, y: float) -> float:
    """Emits a warning on division by zero."""
    if y == 0:
        warnings.warn("division by zero", RuntimeWarning, stacklevel=2)
        return float("inf")
    return x / y


def shout(text: str) -> str:
    return text.upper()


def fetch_remote(url: str) -> dict:
    """Always raises — used to demonstrate the 'errored' bucket."""
    raise ConnectionError(f"network unavailable: cannot reach {url}")
