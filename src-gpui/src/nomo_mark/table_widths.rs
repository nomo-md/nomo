//! 原生自动表格布局：短表按内容收窄，长表适配容器，自动列宽不写回源码。
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub column: usize,
    pub span: usize,
    pub header: bool,
    pub natural: f32,
    /// 按正常断点换行所需的最小内容宽度，不计紧急拆词。
    pub minimum: f32,
    pub chrome: f32,
    pub explicit: Vec<f32>,
}
#[derive(Clone, Copy, Debug)]
pub struct Constraint {
    pub minimum: f32,
    pub preferred: f32,
}

pub fn constraints(count: usize, cells: &[CellMetrics], zoom: f32) -> Vec<Constraint> {
    let mut columns = vec![Constraint { minimum: 0.0, preferred: 0.0 }; count];
    // 32px 是普通单元格含边框与内边距的下限；表头只服从内容本身。
    let bounds = |cell: &CellMetrics| {
        let minimum = (cell.chrome + cell.minimum).max(if cell.header { 0.0 } else { 32.0 * zoom });
        Constraint { minimum, preferred: minimum.max(cell.chrome + cell.natural) }
    };
    for cell in cells {
        if cell.span == 1 {
            if let Some(column) = columns.get_mut(cell.column) {
                let own = bounds(cell);
                column.minimum = column.minimum.max(own.minimum);
                column.preferred = column.preferred.max(own.preferred);
            }
        }
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
        let own = bounds(cell);
        raise(&mut columns, cell, own.minimum, false);
        raise(&mut columns, cell, own.preferred, true);
    }
    // 向上量化下限，避免最终列宽的百分之一像素舍入触发额外拆词。
    for column in &mut columns {
        column.minimum = (column.minimum * 100.0).ceil() / 100.0;
        column.preferred = column.preferred.max(column.minimum);
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
    let natural: f32 = group.iter().map(|c| c.preferred).sum();
    let elastic = !preferred && natural > total;
    let within_natural = required.min(natural);
    let count = group.len();
    for c in group {
        let increment = if elastic {
            (within_natural - total) * (c.preferred - c.minimum) / (natural - total)
                + (required - natural).max(0.0) * c.preferred / natural
        } else {
            (required - total) * if natural > 0.0 {
                c.preferred / natural
            } else {
                1.0 / count as f32
            }
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
    // 自然宽度已经能放下时保持紧凑，不把正文中的空余空间灌入短表。
    let target = minimum.max(available.max(0.0).min(preferred));
    let ratio = if preferred > minimum {
        (target - minimum) / (preferred - minimum)
    } else {
        0.0
    };
    let mut cumulative = 0.0_f64;
    let mut previous = 0.0_f64;
    columns
        .iter()
        .map(|column| {
            cumulative += f64::from(column.minimum + ratio * (column.preferred - column.minimum));
            let rounded = (cumulative * 100.0).round() / 100.0;
            let width = ((rounded - previous) * 100.0).round() / 100.0;
            previous = rounded;
            width as f32
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
            minimum: natural.min(14.0),
            chrome: 23.0,
            explicit: vec![],
        }
    }
    #[test]
    fn auto_width_policy_keeps_short_tables_compact_and_preserves_content_minima() {
        for count in [2, 6] {
            let cells: Vec<_> = (0..count).map(|column| cell(column, 28.0)).collect();
            let result = constraints(count, &cells, 1.0);
            for available in [300.0, 800.0, 1200.0] {
                let widths = allocate(&result, available);
                let expected = available.min(51.0 * count as f32);
                assert!((widths.iter().sum::<f32>() - expected).abs() < 0.02);
                assert!(widths.iter().all(|width| (*width - expected / count as f32).abs() < 0.02));
            }
        }
        // 场景名与数值这类不等长的短列，也不随正文变宽而被拉开。
        let short = constraints(2, &[cell(0, 180.0), cell(1, 52.0)], 1.0);
        assert_eq!(allocate(&short, 800.0), vec![203.0, 75.0]);
        assert_eq!(allocate(&short, 1200.0), vec![203.0, 75.0]);
        assert!((allocate(&short, 200.0).iter().sum::<f32>() - 200.0).abs() < 0.02);
        let empty = constraints(2, &[cell(0, 0.0), cell(1, 0.0)], 1.0);
        assert_eq!(allocate(&empty, 800.0), vec![32.0, 32.0]);
        assert_eq!(allocate(&empty, 20.0), vec![32.0, 32.0]);
        assert_eq!(allocate(&constraints(2, &[], 1.0), 800.0), vec![0.0, 0.0]);
        let mut cells = vec![cell(0, 40.0); 19];
        cells.push(cell(0, 10_000.0));
        cells.push(cell(1, 350.0));
        let result = constraints(2, &cells, 1.0);
        assert_eq!(result[0].preferred, 10_023.0);
        let widths = allocate(&result, 800.0);
        assert!(widths[0] > widths[1]);
        assert!((widths.iter().sum::<f32>() - 800.0).abs() < 0.02);
        let mut unbroken = cell(1, 1600.0);
        unbroken.minimum = unbroken.natural;
        let result = constraints(2, &[cell(0, 28.0), unbroken], 1.0);
        let widths = allocate(&result, 800.0);
        assert!(widths.iter().sum::<f32>() > 800.0);
        assert!(widths[1] >= 1623.0);
        let mut header = cell(0, 140.0);
        header.header = true;
        header.minimum = 140.0;
        let result = constraints(2, &[header, cell(0, 28.0), cell(1, 500.0)], 1.0);
        assert_eq!(allocate(&result, 320.0), vec![163.0, 157.0]);
        let mut merged = cell(0, 280.0);
        merged.header = true;
        merged.span = 2;
        merged.minimum = 280.0;
        let result = constraints(
            3,
            &[merged, cell(0, 28.0), cell(1, 28.0), cell(2, 500.0)],
            1.0,
        );
        let widths = allocate(&result, 430.0);
        assert!(widths[0] + widths[1] >= 303.0);
        assert!((widths.iter().sum::<f32>() - 430.0).abs() < 0.01);
        let mut authored = cell(0, 28.0);
        authored.explicit = vec![250.0];
        let cells = [authored, cell(1, 500.0)];
        let result = constraints(2, &cells, 1.0);
        assert!(allocate(&result, 800.0)[0] >= 250.0);
        assert_eq!(cells[0].explicit, vec![250.0]);
    }
}
