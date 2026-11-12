"""Bounded JSON graph primitives shared by cleanup and contour validation."""

from .validation import array_value, finite_number, identifier_value, object_value


def unique_items(value, name):
    items = array_value(value, name)
    seen = set()
    for item in items:
        item = object_value(item, name)
        item_id = identifier_value(item.get("id"), name + ".id")
        if item_id in seen:
            raise ValueError(f"{name} contains duplicate IDs")
        seen.add(item_id)
    return items


def path_points(value, closed, width, height):
    points = array_value(value, "contour points")
    if len(points) < (3 if closed else 2):
        raise ValueError("contour contains too few points")
    result = []
    for point in points:
        if not isinstance(point, list) or len(point) != 2:
            raise ValueError("contour point must contain x and y")
        x, y = (finite_number(v, "contour coordinate") for v in point)
        if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
            raise ValueError("contour coordinate lies outside source pixel centers")
        result.append((x, y))
    if all(point == result[0] for point in result):
        raise ValueError("contour must contain a nonzero segment")
    return tuple(result)


def reference_ids(value, name):
    references = tuple(identifier_value(item, name) for item in array_value(value, name))
    if len(set(references)) != len(references):
        raise ValueError(f"{name} contains duplicate references")
    return references
