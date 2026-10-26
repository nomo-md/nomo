//! 对齐 src/lib/table-layout/widths.ts 的纯列宽策略；自动列宽不写回源码。
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub column: usize,
    pub span: usize,
    pub header: bool,
    pub natural: f32,
    pub atomic: f32,
    pub chrome: f32,
    pub explicit: Vec<f32>,
}
#[derive(Clone, Copy, Debug)]
pub struct Constraint {
    pub minimum: f32,
    pub preferred: f32,
}

pub fn constraints(count: usize, cells: &[CellMetrics], em: f32) -> Vec<Constraint> {
    let mut columns = Vec::with_capacity(count);
    let mut groups = vec![Vec::new(); count];
    for cell in cells {
        if cell.span == 1 {
            if let Some(group) = groups.get_mut(cell.column) {
                group.push(cell);
            }
        }
    }
    for own in groups {
        let maximum = |field: fn(&CellMetrics) -> f32| {
            own.iter().map(|cell| field(cell)).fold(0.0_f32, f32::max)
        };
        let chrome = maximum(|cell| cell.chrome);
        let natural = maximum(|cell| cell.natural);
        let header = maximum(|cell| if cell.header { cell.natural } else { 0.0 });
        let mut body: Vec<_> = own
            .iter()
            .filter(|cell| !cell.header && cell.natural > 0.0)
            .map(|cell| cell.natural)
            .collect();
        body.sort_by(f32::total_cmp);
        let p90 = body
            .get((body.len() * 9).div_ceil(10).saturating_sub(1))
            .copied()
            .unwrap_or(0.0);
        let longest = body.last().copied().unwrap_or(0.0);
        let minimum = chrome
            + (3.0 * em)
                .max(natural.min(6.0 * em))
                .max(header.min(12.0 * em))
                .max(maximum(|cell| cell.atomic));
        let preferred = minimum.max(
            chrome
                + header
                    .min(12.0 * em)
                    .max(p90.min(28.0 * em))
                    .max(longest.min(12.0 * em)),
        );
        columns.push(Constraint { minimum, preferred });
    }
    for cell in cells {
        for (offset, width) in cell.explicit.iter().enumerate() {
            if width.is_finite() && *width > 0.0 {
                if let Some(column) = columns.get_mut(cell.column + offset) {
                    column.minimum = column.minimum.max(*width);
                    column.preferred = column.preferred.max(column.minimum);
                }
            }
        }
    }
    let mut spans: Vec<_> = cells.iter().filter(|cell| cell.span > 1).collect();
    spans.sort_by_key(|cell| cell.span);
    for cell in spans {
        let minimum = cell.chrome
            + (3.0 * em * cell.span as f32)
                .max(
                    cell.natural
                        .min(if cell.header { 12.0 } else { 6.0 } * em * cell.span as f32),
                )
                .max(cell.atomic);
        raise(&mut columns, cell, minimum, false);
        raise(
            &mut columns,
            cell,
            minimum.max(
                cell.chrome
                    + cell
                        .natural
                        .min(if cell.header { 12.0 } else { 28.0 } * em * cell.span as f32),
            ),
            true,
        );
    }
    columns
}
fn raise(columns: &mut [Constraint], cell: &CellMetrics, required: f32, preferred: bool) {
    let end = cell.column.saturating_add(cell.span).min(columns.len());
    let Some(group) = columns.get_mut(cell.column..end) else {
        return;
    };
    let total: f32 = group
        .iter()
        .map(|c| if preferred { c.preferred } else { c.minimum })
        .sum();
    if total >= required || group.is_empty() {
        return;
    }
    let weight: f32 = group.iter().map(|c| c.preferred).sum();
    let count = group.len();
    for c in group {
        let increment = (required - total)
            * if weight > 0.0 {
                c.preferred / weight
            } else {
                1.0 / count as f32
            };
        if preferred {
            c.preferred += increment;
        } else {
            c.minimum += increment;
        }
        c.preferred = c.preferred.max(c.minimum);
    }
}
pub fn allocate(columns: &[Constraint], available: f32) -> Vec<f32> {
    let minimum: f32 = columns.iter().map(|c| c.minimum).sum();
    let preferred: f32 = columns.iter().map(|c| c.preferred).sum();
    let target = minimum.max(available.max(0.0).min(preferred));
    let ratio = if preferred > minimum {
        (target - minimum) / (preferred - minimum)
    } else {
        0.0
    };
    let mut cumulative = 0.0;
    let mut previous = 0.0;
    columns
        .iter()
        .map(|column| {
            cumulative += column.minimum + ratio * (column.preferred - column.minimum);
            let rounded = (cumulative * 100.0).round() / 100.0;
            let width = ((rounded - previous) * 100.0).round() / 100.0;
            previous = rounded;
            width
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cell(column: usize, natural: f32) -> CellMetrics {
        CellMetrics {
            column,
            natural,
            span: 1,
            header: false,
            atomic: 0.0,
            chrome: 23.0,
            explicit: vec![],
        }
    }
    #[test]
    fn original_width_policy_preserves_outliers_headers_and_spans() {
        let mut cells = vec![cell(0, 40.0); 19];
        cells.push(cell(0, 10_000.0));
        cells.push(cell(1, 350.0));
        let result = constraints(2, &cells, 14.0);
        assert_eq!(result[0].preferred, 191.0);
        assert!(result[0].preferred < result[1].preferred);
        let mut header = cell(0, 140.0);
        header.header = true;
        let result = constraints(2, &[header, cell(0, 28.0), cell(1, 500.0)], 14.0);
        assert_eq!(allocate(&result, 320.0), vec![163.0, 157.0]);
        let mut merged = cell(0, 280.0);
        merged.header = true;
        merged.span = 2;
        let result = constraints(
            3,
            &[merged, cell(0, 28.0), cell(1, 28.0), cell(2, 500.0)],
            14.0,
        );
        let widths = allocate(&result, 430.0);
        assert!(widths[0] + widths[1] >= 303.0);
        assert!((widths.iter().sum::<f32>() - 430.0).abs() < 0.01);
    }
}
