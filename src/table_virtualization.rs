use std::ops::Range;

pub const DATA_ROW_HEIGHT: f64 = 30.0;
pub const FOCUS_DATA_ROW_HEIGHT: f64 = 108.0;
pub const DEFAULT_VIEWPORT_HEIGHT: f64 = 900.0;
pub const OVERSCAN_ROWS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableViewport {
    pub scroll_top: f64,
    pub height: f64,
}

impl Default for TableViewport {
    fn default() -> Self {
        Self {
            scroll_top: 0.0,
            height: DEFAULT_VIEWPORT_HEIGHT,
        }
    }
}

pub fn visible_row_range(row_count: usize, viewport: TableViewport) -> Range<usize> {
    visible_row_range_with_height(row_count, viewport, DATA_ROW_HEIGHT)
}

pub fn visible_row_range_with_height(
    row_count: usize,
    viewport: TableViewport,
    row_height: f64,
) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }

    let scroll_top = if viewport.scroll_top.is_finite() {
        viewport.scroll_top.max(0.0)
    } else {
        0.0
    };
    let height = if viewport.height.is_finite() && viewport.height > 0.0 {
        viewport.height
    } else {
        DEFAULT_VIEWPORT_HEIGHT
    };
    let row_height = if row_height.is_finite() && row_height > 0.0 {
        row_height
    } else {
        DATA_ROW_HEIGHT
    };
    let first_visible = (scroll_top / row_height).floor() as usize;
    let visible_count = (height / row_height).ceil() as usize + 1;
    let start = first_visible.saturating_sub(OVERSCAN_ROWS).min(row_count);
    let end = first_visible
        .saturating_add(visible_count)
        .saturating_add(OVERSCAN_ROWS)
        .min(row_count);

    start..end.max(start)
}

pub fn spacer_heights(row_count: usize, range: &Range<usize>) -> (f64, f64) {
    spacer_heights_with_height(row_count, range, DATA_ROW_HEIGHT)
}

pub fn spacer_heights_with_height(
    row_count: usize,
    range: &Range<usize>,
    row_height: f64,
) -> (f64, f64) {
    let row_height = if row_height.is_finite() && row_height > 0.0 {
        row_height
    } else {
        DATA_ROW_HEIGHT
    };
    let top = range.start.min(row_count) as f64 * row_height;
    let bottom = row_count.saturating_sub(range.end.min(row_count)) as f64 * row_height;
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tables_render_every_row() {
        let range = visible_row_range(12, TableViewport::default());

        assert_eq!(range, 0..12);
        assert_eq!(spacer_heights(12, &range), (0.0, 0.0));
    }

    #[test]
    fn large_tables_only_render_the_window_and_overscan() {
        let range = visible_row_range(
            500_000,
            TableViewport {
                scroll_top: 30_000.0,
                height: 600.0,
            },
        );

        assert_eq!(range, 992..1029);
        assert_eq!(spacer_heights(500_000, &range), (29_760.0, 14_969_130.0));
    }

    #[test]
    fn bottom_window_is_clamped_to_the_available_rows() {
        let range = visible_row_range(
            100,
            TableViewport {
                scroll_top: 30_000.0,
                height: 600.0,
            },
        );

        assert_eq!(range, 100..100);
        assert_eq!(spacer_heights(100, &range), (3_000.0, 0.0));
    }

    #[test]
    fn invalid_viewport_values_fall_back_to_the_initial_window() {
        assert_eq!(
            visible_row_range(
                100,
                TableViewport {
                    scroll_top: f64::NAN,
                    height: 0.0,
                },
            ),
            visible_row_range(100, TableViewport::default())
        );
    }

    #[test]
    fn focused_rows_use_their_expanded_height_for_windowing() {
        let viewport = TableViewport {
            scroll_top: 10_800.0,
            height: 540.0,
        };
        let range = visible_row_range_with_height(10_000, viewport, FOCUS_DATA_ROW_HEIGHT);

        assert_eq!(range, 92..114);
        assert_eq!(
            spacer_heights_with_height(10_000, &range, FOCUS_DATA_ROW_HEIGHT),
            (9_936.0, 1_067_688.0)
        );
    }
}
