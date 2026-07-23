use std::ops::Range;

pub const DATA_ROW_HEIGHT: f64 = 30.0;
pub const DEFAULT_VIEWPORT_HEIGHT: f64 = 900.0;
pub const DEFAULT_VIEWPORT_WIDTH: f64 = 1280.0;
pub const OVERSCAN_ROWS: usize = 8;
pub const ROW_NUMBER_COLUMN_WIDTH: usize = 58;
pub const FOCUS_NEIGHBOR_BASE_WIDTH: usize = 180;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableViewport {
    pub scroll_top: f64,
    pub height: f64,
    pub width: f64,
    pub expanded_row: Option<ExpandedRow>,
}

impl Default for TableViewport {
    fn default() -> Self {
        Self {
            scroll_top: 0.0,
            height: DEFAULT_VIEWPORT_HEIGHT,
            width: DEFAULT_VIEWPORT_WIDTH,
            expanded_row: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpandedRow {
    pub index: usize,
    pub column_index: usize,
    pub height: f64,
}

impl ExpandedRow {
    pub fn measured(
        index: usize,
        column_index: usize,
        content_height: f64,
        viewport_height: f64,
    ) -> Self {
        let maximum = if viewport_height.is_finite() && viewport_height > 0.0 {
            (viewport_height / 3.0).max(DATA_ROW_HEIGHT)
        } else {
            DEFAULT_VIEWPORT_HEIGHT / 3.0
        };
        let height = if content_height.is_finite() {
            content_height.clamp(DATA_ROW_HEIGHT, maximum)
        } else {
            DATA_ROW_HEIGHT
        };
        Self {
            index,
            column_index,
            height,
        }
    }

    pub const fn matches_cell(self, index: usize, column_index: usize) -> bool {
        self.index == index && self.column_index == column_index
    }

    fn extra_height(self, row_count: usize) -> f64 {
        if self.index < row_count {
            (self.height - DATA_ROW_HEIGHT).max(0.0)
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusColumnRole {
    Hidden,
    LeftNeighbor,
    Focused,
    RightNeighbor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusLayout {
    pub focused_index: usize,
    pub focused_width: usize,
    pub table_width: usize,
    pub left_start: usize,
    pub right_end: usize,
    pub left_neighbor_width: usize,
    pub right_neighbor_width: usize,
    pub left_width_remainder: usize,
    pub right_width_remainder: usize,
    pub left_spacer_width: usize,
    pub right_spacer_width: usize,
}

impl FocusLayout {
    pub fn calculate(
        column_count: usize,
        focused_index: usize,
        requested_focus_width: usize,
        viewport_width: f64,
    ) -> Option<Self> {
        if column_count == 0 || focused_index >= column_count {
            return None;
        }

        let viewport_width = if viewport_width.is_finite() && viewport_width > 0.0 {
            viewport_width.round() as usize
        } else {
            DEFAULT_VIEWPORT_WIDTH as usize
        };
        let minimum_table_width = ROW_NUMBER_COLUMN_WIDTH * 2 + 1;
        let table_width = viewport_width.max(minimum_table_width);
        let available_focus_width = table_width - ROW_NUMBER_COLUMN_WIDTH * 2;
        let focused_width = requested_focus_width
            .clamp(320, 720)
            .min(available_focus_width);
        let focus_left = (table_width - focused_width) / 2;
        let left_budget = focus_left.saturating_sub(ROW_NUMBER_COLUMN_WIDTH);
        let right_budget = table_width - focus_left - focused_width;

        let left_capacity = left_budget / FOCUS_NEIGHBOR_BASE_WIDTH;
        let right_capacity = right_budget / FOCUS_NEIGHBOR_BASE_WIDTH;
        let left_count = focused_index.min(left_capacity);
        let right_available = column_count - focused_index - 1;
        let right_count = right_available.min(right_capacity);

        Some(Self {
            focused_index,
            focused_width,
            table_width,
            left_start: focused_index - left_count,
            right_end: focused_index + 1 + right_count,
            left_neighbor_width: left_budget.checked_div(left_count).unwrap_or(0),
            right_neighbor_width: right_budget.checked_div(right_count).unwrap_or(0),
            left_width_remainder: if left_count == 0 {
                0
            } else {
                left_budget % left_count
            },
            right_width_remainder: if right_count == 0 {
                0
            } else {
                right_budget % right_count
            },
            left_spacer_width: if left_count == 0 { left_budget } else { 0 },
            right_spacer_width: if right_count == 0 { right_budget } else { 0 },
        })
    }

    pub fn column_role(&self, column_index: usize) -> FocusColumnRole {
        if column_index == self.focused_index {
            FocusColumnRole::Focused
        } else if (self.left_start..self.focused_index).contains(&column_index) {
            FocusColumnRole::LeftNeighbor
        } else if (self.focused_index + 1..self.right_end).contains(&column_index) {
            FocusColumnRole::RightNeighbor
        } else {
            FocusColumnRole::Hidden
        }
    }

    pub fn column_width(&self, column_index: usize) -> Option<usize> {
        match self.column_role(column_index) {
            FocusColumnRole::Hidden => None,
            FocusColumnRole::Focused => Some(self.focused_width),
            FocusColumnRole::LeftNeighbor => {
                let distance_from_focus = self.focused_index - column_index - 1;
                Some(
                    self.left_neighbor_width
                        + usize::from(distance_from_focus < self.left_width_remainder),
                )
            }
            FocusColumnRole::RightNeighbor => {
                let distance_from_focus = column_index - self.focused_index - 1;
                Some(
                    self.right_neighbor_width
                        + usize::from(distance_from_focus < self.right_width_remainder),
                )
            }
        }
    }
}

pub fn visible_row_range(row_count: usize, viewport: TableViewport) -> Range<usize> {
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
    let expanded_row = viewport.expanded_row.filter(|row| row.index < row_count);
    if scroll_top >= row_offset(row_count, row_count, expanded_row) {
        return row_count..row_count;
    }
    let first_visible = row_at_offset(row_count, scroll_top, expanded_row);
    let last_visible = row_at_offset(row_count, scroll_top + height, expanded_row);
    let start = first_visible.saturating_sub(OVERSCAN_ROWS).min(row_count);
    let end = last_visible
        .saturating_add(1)
        .saturating_add(OVERSCAN_ROWS)
        .min(row_count);

    start..end.max(start)
}

pub fn spacer_heights(
    row_count: usize,
    range: &Range<usize>,
    expanded_row: Option<ExpandedRow>,
) -> (f64, f64) {
    let expanded_row = expanded_row.filter(|row| row.index < row_count);
    let top = row_offset(range.start.min(row_count), row_count, expanded_row);
    let total = row_offset(row_count, row_count, expanded_row);
    let bottom = total - row_offset(range.end.min(row_count), row_count, expanded_row);
    (top, bottom)
}

pub fn row_offset(row_index: usize, row_count: usize, expanded_row: Option<ExpandedRow>) -> f64 {
    let row_index = row_index.min(row_count);
    let extra = expanded_row
        .filter(|row| row.index < row_index)
        .map_or(0.0, |row| row.extra_height(row_count));
    row_index as f64 * DATA_ROW_HEIGHT + extra
}

fn row_at_offset(row_count: usize, offset: f64, expanded_row: Option<ExpandedRow>) -> usize {
    if row_count == 0 {
        return 0;
    }
    let offset = if offset.is_finite() {
        offset.max(0.0)
    } else {
        0.0
    };
    let Some(expanded) = expanded_row else {
        return ((offset / DATA_ROW_HEIGHT).floor() as usize).min(row_count);
    };
    let expanded_start = expanded.index as f64 * DATA_ROW_HEIGHT;
    if offset < expanded_start {
        return ((offset / DATA_ROW_HEIGHT).floor() as usize).min(row_count);
    }
    if offset < expanded_start + expanded.height {
        return expanded.index;
    }
    let rows_after =
        ((offset - expanded_start - expanded.height) / DATA_ROW_HEIGHT).floor() as usize;
    expanded
        .index
        .saturating_add(1)
        .saturating_add(rows_after)
        .min(row_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tables_render_every_row() {
        let range = visible_row_range(12, TableViewport::default());

        assert_eq!(range, 0..12);
        assert_eq!(spacer_heights(12, &range, None), (0.0, 0.0));
    }

    #[test]
    fn large_tables_only_render_the_window_and_overscan() {
        let range = visible_row_range(
            500_000,
            TableViewport {
                scroll_top: 30_000.0,
                height: 600.0,
                ..TableViewport::default()
            },
        );

        assert_eq!(range, 992..1029);
        assert_eq!(
            spacer_heights(500_000, &range, None),
            (29_760.0, 14_969_130.0)
        );
    }

    #[test]
    fn bottom_window_is_clamped_to_the_available_rows() {
        let range = visible_row_range(
            100,
            TableViewport {
                scroll_top: 30_000.0,
                height: 600.0,
                ..TableViewport::default()
            },
        );

        assert_eq!(range, 100..100);
        assert_eq!(spacer_heights(100, &range, None), (3_000.0, 0.0));
    }

    #[test]
    fn invalid_viewport_values_fall_back_to_the_initial_window() {
        assert_eq!(
            visible_row_range(
                100,
                TableViewport {
                    scroll_top: f64::NAN,
                    height: 0.0,
                    ..TableViewport::default()
                },
            ),
            visible_row_range(100, TableViewport::default())
        );
    }

    #[test]
    fn measured_row_height_keeps_short_content_at_the_base_height_and_caps_long_content() {
        assert_eq!(ExpandedRow::measured(4, 2, 18.0, 600.0).height, 30.0);
        assert_eq!(ExpandedRow::measured(4, 2, 120.0, 600.0).height, 120.0);
        assert_eq!(ExpandedRow::measured(4, 2, 900.0, 600.0).height, 200.0);
        assert_eq!(ExpandedRow::measured(4, 2, f64::NAN, 600.0).height, 30.0);
        assert_eq!(ExpandedRow::measured(4, 2, 900.0, f64::NAN).height, 300.0);
    }

    #[test]
    fn expanded_row_identity_includes_the_focused_column() {
        let expanded = ExpandedRow::measured(4, 2, 120.0, 600.0);

        assert!(expanded.matches_cell(4, 2));
        assert!(!expanded.matches_cell(4, 3));
        assert!(!expanded.matches_cell(5, 2));
    }

    #[test]
    fn a_single_expanded_row_changes_offsets_only_after_that_row() {
        let expanded = Some(ExpandedRow::measured(10, 2, 120.0, 900.0));

        assert_eq!(row_offset(5, 100, expanded), 150.0);
        assert_eq!(row_offset(10, 100, expanded), 300.0);
        assert_eq!(row_offset(11, 100, expanded), 420.0);
        assert_eq!(row_offset(100, 100, expanded), 3_090.0);
    }

    #[test]
    fn expanded_row_windowing_and_spacers_work_before_inside_and_after_the_row() {
        let expanded = Some(ExpandedRow::measured(50, 2, 180.0, 900.0));

        let before = visible_row_range(
            200,
            TableViewport {
                scroll_top: 900.0,
                height: 300.0,
                expanded_row: expanded,
                ..TableViewport::default()
            },
        );
        let inside = visible_row_range(
            200,
            TableViewport {
                scroll_top: 1_520.0,
                height: 100.0,
                expanded_row: expanded,
                ..TableViewport::default()
            },
        );
        let after = visible_row_range(
            200,
            TableViewport {
                scroll_top: 2_250.0,
                height: 300.0,
                expanded_row: expanded,
                ..TableViewport::default()
            },
        );

        assert_eq!(before, 22..49);
        assert_eq!(inside, 42..59);
        assert_eq!(after, 62..89);
        assert_eq!(spacer_heights(200, &inside, expanded), (1_260.0, 4_230.0));
    }

    #[test]
    fn focus_layout_shows_as_many_neighbors_as_each_side_can_fit() {
        let layout = FocusLayout::calculate(30, 15, 400, 1_958.0).unwrap();

        assert_eq!(layout.left_start, 11);
        assert_eq!(layout.right_end, 20);
        assert_eq!(layout.column_role(10), FocusColumnRole::Hidden);
        assert_eq!(layout.column_role(11), FocusColumnRole::LeftNeighbor);
        assert_eq!(layout.column_role(15), FocusColumnRole::Focused);
        assert_eq!(layout.column_role(19), FocusColumnRole::RightNeighbor);
        assert_eq!(layout.column_role(20), FocusColumnRole::Hidden);
    }

    #[test]
    fn focus_layout_has_no_neighbor_count_limit() {
        let layout = FocusLayout::calculate(40, 20, 400, 2_858.0).unwrap();

        assert_eq!(layout.focused_index - layout.left_start, 6);
        assert_eq!(layout.right_end - layout.focused_index - 1, 6);
    }

    #[test]
    fn focus_layout_stretches_the_only_remaining_neighbor() {
        let layout = FocusLayout::calculate(20, 1, 400, 1_500.0).unwrap();

        assert_eq!(layout.left_start, 0);
        assert_eq!(layout.left_neighbor_width, 492);
        assert_eq!(layout.left_spacer_width, 0);
    }

    #[test]
    fn focus_layout_keeps_an_empty_side_blank_and_the_focus_centered() {
        let layout = FocusLayout::calculate(20, 0, 400, 1_500.0).unwrap();

        assert_eq!(layout.left_start, 0);
        assert_eq!(layout.left_spacer_width, 492);
        assert_eq!(layout.table_width - layout.focused_width, 1_100);
        assert_eq!((layout.table_width - layout.focused_width) / 2, 550);
    }

    #[test]
    fn focus_layout_uses_the_default_width_for_invalid_measurements() {
        let layout = FocusLayout::calculate(10, 5, 720, f64::NAN).unwrap();

        assert_eq!(layout.table_width, DEFAULT_VIEWPORT_WIDTH as usize);
        assert_eq!(layout.focused_width, 720);
    }

    #[test]
    fn focus_layout_capacity_grows_from_one_neighbor_without_a_cap() {
        let one = FocusLayout::calculate(30, 15, 400, 1_000.0).unwrap();
        let two = FocusLayout::calculate(30, 15, 400, 1_400.0).unwrap();
        let three = FocusLayout::calculate(30, 15, 400, 1_800.0).unwrap();
        let four = FocusLayout::calculate(30, 15, 400, 2_200.0).unwrap();

        assert_eq!(one.focused_index - one.left_start, 1);
        assert_eq!(two.focused_index - two.left_start, 2);
        assert_eq!(three.focused_index - three.left_start, 3);
        assert_eq!(four.focused_index - four.left_start, 4);
    }

    #[test]
    fn focus_layout_allocates_every_pixel_and_keeps_the_focus_center_exact() {
        let layout = FocusLayout::calculate(30, 15, 401, 1_503.0).unwrap();
        let visible_columns_width = (layout.left_start..layout.right_end)
            .filter_map(|column| layout.column_width(column))
            .sum::<usize>();

        assert_eq!(
            ROW_NUMBER_COLUMN_WIDTH
                + layout.left_spacer_width
                + visible_columns_width
                + layout.right_spacer_width,
            layout.table_width
        );
        let left_width = (layout.left_start..layout.focused_index)
            .filter_map(|column| layout.column_width(column))
            .sum::<usize>();
        assert_eq!(
            ROW_NUMBER_COLUMN_WIDTH + layout.left_spacer_width + left_width,
            (layout.table_width - layout.focused_width) / 2
        );
    }

    #[test]
    fn focus_layout_shrinks_the_focus_when_the_viewport_is_too_narrow() {
        let layout = FocusLayout::calculate(3, 1, 720, 200.0).unwrap();

        assert_eq!(layout.table_width, 200);
        assert_eq!(layout.focused_width, 84);
        assert_eq!(layout.left_start, 1);
        assert_eq!(layout.right_end, 2);
    }
}
