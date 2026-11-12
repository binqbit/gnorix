"""Bounded JSON values shared by native report validators."""

import math

MAX_ITEMS = 100_000


def object_value(value, name):
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value, name, limit=MAX_ITEMS):
    if not isinstance(value, list) or len(value) > limit:
        raise ValueError(f"{name} must be an array with at most {limit} items")
    return value


def finite_number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a finite number")
    try:
        number = float(value)
    except OverflowError as error:
        raise ValueError(f"{name} must be a finite number") from error
    if not math.isfinite(number):
        raise ValueError(f"{name} must be a finite number")
    return number


def require_close(actual, expected, name):
    if not math.isclose(actual, expected, rel_tol=1e-8, abs_tol=1e-7):
        raise ValueError(f"{name} does not match its source evidence")


def identifier_value(value, name):
    if type(value) is not int or not 0 <= value < 2**64:
        raise ValueError(f"{name} must be a nonnegative 64-bit integer")
    return value
