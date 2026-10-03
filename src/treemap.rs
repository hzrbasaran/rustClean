//! Squarified treemap layout (Bruls, Huizing, van Wijk) for terminal cells.
//!
//! A terminal cell is about twice as tall as it is wide, so the layout is
//! computed with each row counting as two units and rounded back to cells.
//! Rectangles share their edges exactly: they never overlap and together
//! fill the whole area.

use ratatui::layout::Rect;

/// Blocks smaller than this many cells are merged into one "other" block.
pub const MIN_CELLS: f64 = 6.0;
/// At most this many separate blocks.
pub const MAX_BLOCKS: usize = 80;

/// What a block stands for: an input index, or all the small leftovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Item(usize),
    Other { count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Lays out `sizes` (any order) in `area`. Zero sizes are left out.
pub fn layout(sizes: &[u64], area: Rect) -> Vec<(Slot, Rect)> {
    let total: u64 = sizes.iter().sum();
    if total == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let cells = f64::from(area.width) * f64::from(area.height);
    let mut order: Vec<usize> = (0..sizes.len()).filter(|&i| sizes[i] > 0).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));

    // Separate blocks for everything big enough; the rest becomes "other".
    let cell_area = |size: u64| size as f64 / total as f64 * cells;
    let keep = order
        .iter()
        .take(MAX_BLOCKS)
        .take_while(|&&i| cell_area(sizes[i]) >= MIN_CELLS)
        .count();
    let items: Vec<(Slot, f64)> = order[..keep]
        .iter()
        .map(|&i| (Slot::Item(i), sizes[i] as f64))
        .collect();
    // Small items share a full-width strip along the bottom: always
    // visible, however little they weigh.
    let rest = &order[keep..];
    let mut area = area;
    let mut other = None;
    if !rest.is_empty() && area.height >= 3 {
        area.height -= 1;
        let strip = Rect::new(area.x, area.y + area.height, area.width, 1);
        other = Some((Slot::Other { count: rest.len() }, strip));
    }

    // Virtual space: rows count double so blocks come out roughly square.
    let space = FRect {
        x: 0.0,
        y: 0.0,
        w: f64::from(area.width),
        h: f64::from(area.height) * 2.0,
    };
    let sum: f64 = items.iter().map(|i| i.1).sum();
    let scale = space.w * space.h / sum;
    let scaled: Vec<(Slot, f64)> = items.into_iter().map(|(k, s)| (k, s * scale)).collect();
    let mut out = Vec::new();
    squarify(&scaled, space, &mut out);

    out.into_iter()
        .filter_map(|(slot, r)| {
            let x0 = r.x.round() as u16;
            let x1 = (r.x + r.w).round() as u16;
            let y0 = (r.y / 2.0).round() as u16;
            let y1 = ((r.y + r.h) / 2.0).round() as u16;
            (x1 > x0 && y1 > y0).then(|| {
                let rect = Rect::new(area.x + x0, area.y + y0, x1 - x0, y1 - y0);
                (slot, rect)
            })
        })
        .chain(other)
        .collect()
}

/// Worst aspect ratio of a row of areas laid along a side of length `side`.
fn worst(row: &[f64], side: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    let (min, max) = row
        .iter()
        .fold((f64::MAX, 0f64), |(lo, hi), &a| (lo.min(a), hi.max(a)));
    let s2 = sum * sum;
    let w2 = side * side;
    (w2 * max / s2).max(s2 / (w2 * min))
}

fn squarify(items: &[(Slot, f64)], mut rect: FRect, out: &mut Vec<(Slot, FRect)>) {
    let mut start = 0;
    while start < items.len() {
        let side = rect.w.min(rect.h);
        // Grow the row while it makes the blocks squarer.
        let mut end = start + 1;
        while end < items.len() {
            let areas = |e: usize| items[start..e].iter().map(|i| i.1).collect::<Vec<_>>();
            if worst(&areas(end + 1), side) > worst(&areas(end), side) {
                break;
            }
            end += 1;
        }
        let last_row = end == items.len();
        rect = place_row(&items[start..end], rect, last_row, out);
        start = end;
    }
}

/// Places a row along the shorter side of `rect`; returns what is left.
fn place_row(row: &[(Slot, f64)], rect: FRect, last: bool, out: &mut Vec<(Slot, FRect)>) -> FRect {
    let sum: f64 = row.iter().map(|i| i.1).sum();
    if rect.w >= rect.h {
        // A column on the left.
        let width = if last { rect.w } else { sum / rect.h };
        let mut y = rect.y;
        for (i, (slot, area)) in row.iter().enumerate() {
            let h = if i + 1 == row.len() {
                rect.y + rect.h - y
            } else {
                area / width
            };
            out.push((
                *slot,
                FRect {
                    x: rect.x,
                    y,
                    w: width,
                    h,
                },
            ));
            y += h;
        }
        FRect {
            x: rect.x + width,
            y: rect.y,
            w: rect.w - width,
            h: rect.h,
        }
    } else {
        // A row along the top.
        let height = if last { rect.h } else { sum / rect.w };
        let mut x = rect.x;
        for (i, (slot, area)) in row.iter().enumerate() {
            let w = if i + 1 == row.len() {
                rect.x + rect.w - x
            } else {
                area / height
            };
            out.push((
                *slot,
                FRect {
                    x,
                    y: rect.y,
                    w,
                    h: height,
                },
            ));
            x += w;
        }
        FRect {
            x: rect.x,
            y: rect.y + height,
            w: rect.w,
            h: rect.h - height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The block to move to from `from` in direction `dir`: the closest one
/// on that side, preferring blocks that line up with the current one, and
/// among equals the top-most / left-most (reading order).
pub fn neighbor(rects: &[Rect], from: usize, dir: Dir) -> Option<usize> {
    let r = rects.get(from)?;
    rects
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != from)
        .filter_map(|(i, o)| {
            // Gap between the facing edges, and whether the blocks overlap
            // across the direction of movement.
            let (gap, overlap, start) = match dir {
                Dir::Right => (
                    o.x.checked_sub(r.right())?,
                    spans(o.y, o.bottom(), r.y, r.bottom()),
                    o.y,
                ),
                Dir::Left => (
                    r.x.checked_sub(o.right())?,
                    spans(o.y, o.bottom(), r.y, r.bottom()),
                    o.y,
                ),
                Dir::Down => (
                    o.y.checked_sub(r.bottom())?,
                    spans(o.x, o.right(), r.x, r.right()),
                    o.x,
                ),
                Dir::Up => (
                    r.y.checked_sub(o.bottom())?,
                    spans(o.x, o.right(), r.x, r.right()),
                    o.x,
                ),
            };
            Some((i, (gap, !overlap, start)))
        })
        .min_by_key(|&(_, key)| key)
        .map(|(i, _)| i)
}

/// Whether the ranges `[a0, a1)` and `[b0, b1)` overlap.
fn spans(a0: u16, a1: u16, b0: u16, b1: u16) -> bool {
    a0 < b1 && b0 < a1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_tiling(blocks: &[(Slot, Rect)], area: Rect) {
        let mut covered = vec![0u8; usize::from(area.width) * usize::from(area.height)];
        for (_, r) in blocks {
            assert!(r.x >= area.x && r.y >= area.y, "{r:?} outside {area:?}");
            assert!(
                r.right() <= area.right() && r.bottom() <= area.bottom(),
                "{r:?} outside"
            );
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    covered[usize::from(y - area.y) * usize::from(area.width)
                        + usize::from(x - area.x)] += 1;
                }
            }
        }
        assert!(covered.iter().all(|&c| c == 1), "gaps or overlaps");
    }

    #[test]
    fn tiles_the_area_proportionally() {
        let area = Rect::new(2, 3, 120, 36);
        let sizes = [500, 300, 120, 50, 20, 10];
        let blocks = layout(&sizes, area);
        check_tiling(&blocks, area);
        let total: u64 = sizes.iter().sum();
        let cells = f64::from(area.width) * f64::from(area.height);
        for (slot, r) in &blocks {
            let Slot::Item(i) = slot else {
                panic!("no small items here")
            };
            let want = sizes[*i] as f64 / total as f64 * cells;
            let got = f64::from(r.width) * f64::from(r.height);
            // Rounding to whole cells costs at most about one row and column.
            let slack = f64::from(r.width) + 2.0 * f64::from(r.height) + 2.0;
            assert!((got - want).abs() <= slack, "item {i}: {got} vs {want}");
        }
        // The biggest item gets the biggest block.
        let biggest = blocks.iter().max_by_key(|(_, r)| r.area()).unwrap();
        assert_eq!(biggest.0, Slot::Item(0));
    }

    #[test]
    fn blocks_are_roughly_square() {
        let area = Rect::new(0, 0, 100, 30);
        let blocks = layout(&[1; 9], area);
        for (_, r) in &blocks {
            // Cells are 1:2, so width ≈ 2 × height looks square.
            let ratio = f64::from(r.width) / (2.0 * f64::from(r.height));
            assert!((0.4..=2.5).contains(&ratio), "{r:?}");
        }
    }

    #[test]
    fn small_items_merge_into_other() {
        let area = Rect::new(0, 0, 40, 10);
        let mut sizes = vec![10_000, 5_000];
        sizes.extend([1; 30]);
        sizes.push(0);
        let blocks = layout(&sizes, area);
        check_tiling(&blocks, area);
        assert!(blocks.iter().any(|(s, _)| *s == Slot::Other { count: 30 }));
        assert_eq!(blocks.len(), 3);
        assert!(layout(&[0, 0], area).is_empty());
    }

    #[test]
    fn moves_to_neighbors() {
        // ┌──┬──┐
        // │0 │1 │
        // ├──┼──┤
        // │2 │3 │
        // └──┴──┘
        let rects = [
            Rect::new(0, 0, 10, 5),
            Rect::new(10, 0, 10, 5),
            Rect::new(0, 5, 10, 5),
            Rect::new(10, 5, 10, 5),
        ];
        assert_eq!(neighbor(&rects, 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&rects, 0, Dir::Down), Some(2));
        assert_eq!(neighbor(&rects, 3, Dir::Up), Some(1));
        assert_eq!(neighbor(&rects, 3, Dir::Left), Some(2));
        assert_eq!(neighbor(&rects, 0, Dir::Left), None);
        assert_eq!(neighbor(&rects, 0, Dir::Up), None);

        // ┌────┬──┐  From the tall block, → goes to the top one (reading
        // │    │1 │  order), and from 2, ↑ goes back to 1, ← to 0.
        // │ 0  ├──┤
        // │    │2 │
        // └────┴──┘
        let rects = [
            Rect::new(0, 0, 20, 10),
            Rect::new(20, 0, 10, 4),
            Rect::new(20, 4, 10, 6),
        ];
        assert_eq!(neighbor(&rects, 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&rects, 2, Dir::Up), Some(1));
        assert_eq!(neighbor(&rects, 2, Dir::Left), Some(0));
        assert_eq!(neighbor(&rects, 1, Dir::Down), Some(2));
    }
}
