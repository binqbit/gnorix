//! Source-grid traversal shared by line photometry and support checks.

use crate::biometrics::geometry::Point;

/// A source interpolation cell over an interval of segment parameter `t`.
/// Zero-duration intervals require support validation but contribute no integral.
#[derive(Clone, Copy, Debug)]
pub(super) struct CellInterval {
    pub x: usize,
    pub y: usize,
    pub start: f64,
    pub end: f64,
}

/// Visits every bilinear cell touched by an in-bounds segment, in parameter order.
/// Cell identity follows integer grid events, never a rounded floating midpoint.
/// Nearly simultaneous axis events also visit both possible side cells: floating
/// division cannot reliably resolve which side of that vertex was crossed first.
/// Work and allocation are bounded by the source width plus height.
pub(super) fn visit_cells(
    start: Point,
    end: Point,
    width: usize,
    height: usize,
    mut visit: impl FnMut(CellInterval) -> Option<()>,
) -> Option<()> {
    if width == 0 || height == 0 {
        return None;
    }
    for point in [start, end] {
        if point.x() < 0.0
            || point.y() < 0.0
            || point.x() > (width - 1) as f64
            || point.y() > (height - 1) as f64
        {
            return None;
        }
    }

    let (mut x, mut y) = (
        first_cell(start.x(), end.x()),
        first_cell(start.y(), end.y()),
    );
    if start == end {
        return visit(CellInterval {
            x,
            y,
            start: 0.0,
            end: 0.0,
        });
    }

    let crossings = crossings(start, end);
    let (mut previous, mut cursor) = (0.0, 0);
    while cursor < crossings.len() {
        let time = crossings[cursor].time;
        visit(CellInterval {
            x,
            y,
            start: previous,
            end: time,
        })?;

        let (mut next_x, mut next_y) = (x, y);
        while cursor < crossings.len() && crossings[cursor].time == time {
            next_x = crossings[cursor].x.unwrap_or(next_x);
            next_y = crossings[cursor].y.unwrap_or(next_y);
            cursor += 1;
        }

        let (mut possible_x, mut possible_y) = (next_x, next_y);
        for crossing in &crossings[cursor..] {
            // A small relative arithmetic allowance covers the subtraction and
            // division used to compute each event time, without hiding any span.
            let uncertainty = 8.0 * f64::EPSILON * time.abs().max(crossing.time.abs());
            if crossing.time - time > uncertainty {
                break;
            }
            possible_x = crossing.x.unwrap_or(possible_x);
            possible_y = crossing.y.unwrap_or(possible_y);
        }
        if possible_x != x && possible_y != y {
            visit(CellInterval {
                x: possible_x,
                y,
                start: time,
                end: time,
            })?;
            visit(CellInterval {
                x,
                y: possible_y,
                start: time,
                end: time,
            })?;
        }
        if time == 1.0 && (next_x != x || next_y != y) {
            // A strictly internal crossing can round to t=1. Its final cell
            // still contains source evidence, even when its duration rounds away.
            visit(CellInterval {
                x: next_x,
                y: next_y,
                start: time,
                end: time,
            })?;
        }

        (x, y, previous) = (next_x, next_y, time);
    }
    Some(())
}

struct Crossing {
    time: f64,
    x: Option<usize>,
    y: Option<usize>,
}

fn crossings(start: Point, end: Point) -> Vec<Crossing> {
    let mut crossings = vec![Crossing {
        time: 1.0,
        x: None,
        y: None,
    }];
    for (horizontal, a, b) in [(true, start.x(), end.x()), (false, start.y(), end.y())] {
        if a == b {
            continue;
        }
        for boundary in (a.min(b).floor() as usize + 1)..(a.max(b).ceil() as usize) {
            let time = ((boundary as f64 - a) / (b - a)).clamp(0.0, 1.0);
            let cell = if b > a { boundary } else { boundary - 1 };
            crossings.push(Crossing {
                time,
                x: horizontal.then_some(cell),
                y: (!horizontal).then_some(cell),
            });
        }
    }
    crossings.sort_by(|a, b| a.time.total_cmp(&b.time));
    crossings
}

fn first_cell(start: f64, end: f64) -> usize {
    let cell = start.floor() as usize;
    if end < start && start.fract() == 0.0 {
        cell.saturating_sub(1)
    } else {
        cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn visited(start: Point, end: Point) -> Vec<CellInterval> {
        let mut cells = Vec::new();
        visit_cells(start, end, 4, 4, |cell| {
            cells.push(cell);
            Some(())
        })
        .unwrap();
        cells
    }

    #[test]
    fn adjacent_float_events_keep_corner_cells_in_both_directions() {
        let a = point(0.0, 0.0);
        let b = point(1.4999999999999998, 1.5);
        for (start, end) in [(a, b), (b, a)] {
            let cells = visited(start, end);
            assert!(cells.iter().any(|cell| cell.x == 0 && cell.y == 1));
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.x == 1 && cell.y == 0 && cell.start == cell.end)
            );
            assert!(
                (cells.iter().map(|cell| cell.end - cell.start).sum::<f64>() - 1.0).abs() < 1e-15
            );
            assert!(cells.iter().all(|cell| cell.start <= cell.end));
        }
    }

    #[test]
    fn coincident_axis_events_validate_both_possible_side_cells() {
        for (a, b) in [
            (point(0.2, 0.2), point(1.8, 1.8)),
            (point(1.8, 1.8), point(0.2, 0.2)),
        ] {
            let cells = visited(a, b);
            for (x, y) in [(0, 1), (1, 0)] {
                assert!(
                    cells
                        .iter()
                        .any(|cell| cell.x == x && cell.y == y && cell.start == cell.end)
                );
            }
        }
    }

    #[test]
    fn integer_reverse_starts_and_single_pixel_axes_do_not_step_outside() {
        let cells = visited(point(3.0, 2.0), point(0.0, 2.0));
        let nonempty: Vec<_> = cells
            .iter()
            .filter(|cell| cell.start < cell.end)
            .map(|cell| (cell.x, cell.y))
            .collect();
        assert_eq!(nonempty, [(2, 2), (1, 2), (0, 2)]);
        let mut cells = Vec::new();
        visit_cells(point(0.0, 0.0), point(0.0, 3.0), 1, 4, |cell| {
            cells.push(cell);
            Some(())
        })
        .unwrap();
        assert!(cells.iter().all(|cell| cell.x == 0 && cell.y < 4));
        assert_eq!(
            visit_cells(point(-0.1, 0.0), point(0.0, 1.0), 1, 4, |_| Some(())),
            None
        );
        assert_eq!(
            visit_cells(point(0.0, 0.0), point(0.0, 0.0), 0, 4, |_| Some(())),
            None
        );
    }

    #[test]
    fn point_segments_and_rejected_support_are_explicit() {
        let cells = visited(point(1.2, 2.3), point(1.2, 2.3));
        assert_eq!(cells.len(), 1);
        assert_eq!(
            (cells[0].x, cells[0].y, cells[0].start, cells[0].end),
            (1, 2, 0.0, 0.0)
        );
        let rejected = visit_cells(point(0.0, 0.0), point(1.5, 1.5), 4, 4, |cell| {
            (cell.x != 0 || cell.y != 1).then_some(())
        });
        assert_eq!(rejected, None);
    }
}
