"""Bounded JSON graph primitives shared by cleanup and contour validation."""

import math

_MAX_ITEMS = 100_000


def _object(value, name):
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _array(value, name, limit=_MAX_ITEMS):
    if not isinstance(value, list) or len(value) > limit:
        raise ValueError(f"{name} must be an array with at most {limit} items")
    return value


def _number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be finite")
    try:
        number = float(value)
    except OverflowError as error:
        raise ValueError(f"{name} must be finite") from error
    if not math.isfinite(number):
        raise ValueError(f"{name} must be finite")
    return number


def _same(actual, expected, name):
    if not math.isclose(actual, expected, rel_tol=1e-8, abs_tol=1e-7):
        raise ValueError(f"{name} does not match its source evidence")


def _id(value, name):
    if type(value) is not int or not 0 <= value < 2**64:
        raise ValueError(f"{name} must be a nonnegative 64-bit integer")
    return value


def _unique_items(value, name):
    items = _array(value, name)
    seen = set()
    for item in items:
        item = _object(item, name)
        identifier = _id(item.get("id"), name + ".id")
        if identifier in seen:
            raise ValueError(f"{name} contains duplicate IDs")
        seen.add(identifier)
    return items


def _points(value, closed, width, height):
    points = _array(value, "contour points")
    if len(points) < (3 if closed else 2):
        raise ValueError("contour contains too few points")
    result = []
    for point in points:
        if not isinstance(point, list) or len(point) != 2:
            raise ValueError("contour point must contain x and y")
        x, y = (_number(v, "contour coordinate") for v in point)
        if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
            raise ValueError("contour coordinate lies outside source pixel centers")
        result.append((x, y))
    if all(point == result[0] for point in result):
        raise ValueError("contour must contain a nonzero segment")
    return tuple(result)


def _references(value, name):
    references = tuple(_id(item, name) for item in _array(value, name))
    if len(set(references)) != len(references):
        raise ValueError(f"{name} contains duplicate references")
    return references
