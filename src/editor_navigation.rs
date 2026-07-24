use crate::diagnostics::ColumnAnalysis;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridPosition {
    pub row_index: usize,
    pub column_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridMovement {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticTarget {
    Cell(GridPosition),
    Column(usize),
}

pub fn move_in_grid(
    current: Option<GridPosition>,
    header_rows: usize,
    row_count: usize,
    column_count: usize,
    movement: GridMovement,
) -> Option<GridPosition> {
    if row_count <= header_rows || column_count == 0 {
        return None;
    }

    let first_row = header_rows;
    let last_row = row_count - 1;
    let last_column = column_count - 1;
    let mut position = current
        .filter(|position| {
            (first_row..=last_row).contains(&position.row_index)
                && position.column_index <= last_column
        })
        .unwrap_or(GridPosition {
            row_index: first_row,
            column_index: 0,
        });

    match movement {
        GridMovement::Up => {
            position.row_index = position.row_index.saturating_sub(1).max(first_row)
        }
        GridMovement::Down => {
            position.row_index = position.row_index.saturating_add(1).min(last_row)
        }
        GridMovement::Left => position.column_index = position.column_index.saturating_sub(1),
        GridMovement::Right => {
            position.column_index = position.column_index.saturating_add(1).min(last_column)
        }
    }
    Some(position)
}

pub fn diagnostic_targets(analyses: &[ColumnAnalysis]) -> Vec<DiagnosticTarget> {
    let mut cells = analyses
        .iter()
        .enumerate()
        .flat_map(|(column_index, analysis)| {
            analysis.problems.iter().map(move |problem| {
                DiagnosticTarget::Cell(GridPosition {
                    row_index: problem.row_index,
                    column_index,
                })
            })
        })
        .collect::<Vec<_>>();
    cells.sort_by_key(|target| match target {
        DiagnosticTarget::Cell(position) => (position.row_index, position.column_index),
        DiagnosticTarget::Column(column_index) => (usize::MAX, *column_index),
    });
    cells.dedup();

    cells.extend(
        analyses
            .iter()
            .enumerate()
            .filter(|(_, analysis)| analysis.has_mixed_warning)
            .map(|(column_index, _)| DiagnosticTarget::Column(column_index)),
    );
    cells
}

pub fn cycle_diagnostic(
    targets: &[DiagnosticTarget],
    current: Option<DiagnosticTarget>,
    backwards: bool,
) -> Option<DiagnosticTarget> {
    if targets.is_empty() {
        return None;
    }

    let current_index =
        current.and_then(|current| targets.iter().position(|item| *item == current));
    let next_index = match (current_index, backwards) {
        (Some(0), true) | (None, true) => targets.len() - 1,
        (Some(index), true) => index - 1,
        (Some(index), false) => (index + 1) % targets.len(),
        (None, false) => 0,
    };
    Some(targets[next_index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{CellProblem, CellProblemKind, ColumnType};

    fn analysis(
        column_type: ColumnType,
        problem_rows: &[usize],
        has_mixed_warning: bool,
    ) -> ColumnAnalysis {
        ColumnAnalysis {
            column_type,
            type_expression: column_type.label().to_owned(),
            problems: problem_rows
                .iter()
                .map(|row_index| CellProblem {
                    row_index: *row_index,
                    kinds: vec![CellProblemKind::DangerousInvisibleCharacter],
                    detail: None,
                })
                .collect(),
            has_mixed_warning,
            max_content_chars: 0,
        }
    }

    #[test]
    fn grid_movement_stays_inside_data_cells() {
        let first = move_in_grid(None, 2, 5, 3, GridMovement::Left).unwrap();
        assert_eq!(
            first,
            GridPosition {
                row_index: 2,
                column_index: 0
            }
        );

        let top_left = move_in_grid(Some(first), 2, 5, 3, GridMovement::Up).unwrap();
        assert_eq!(top_left, first);

        let bottom_right = move_in_grid(
            Some(GridPosition {
                row_index: 4,
                column_index: 2,
            }),
            2,
            5,
            3,
            GridMovement::Down,
        )
        .unwrap();
        assert_eq!(
            bottom_right,
            GridPosition {
                row_index: 4,
                column_index: 2,
            }
        );
    }

    #[test]
    fn diagnostics_order_cells_by_row_then_column_before_warning_columns() {
        let analyses = vec![
            analysis(ColumnType::String, &[4], true),
            analysis(ColumnType::Json, &[3, 4], false),
        ];

        assert_eq!(
            diagnostic_targets(&analyses),
            vec![
                DiagnosticTarget::Cell(GridPosition {
                    row_index: 3,
                    column_index: 1,
                }),
                DiagnosticTarget::Cell(GridPosition {
                    row_index: 4,
                    column_index: 0,
                }),
                DiagnosticTarget::Cell(GridPosition {
                    row_index: 4,
                    column_index: 1,
                }),
                DiagnosticTarget::Column(0),
            ]
        );
    }

    #[test]
    fn diagnostics_cycle_in_both_directions() {
        let targets = vec![
            DiagnosticTarget::Cell(GridPosition {
                row_index: 2,
                column_index: 0,
            }),
            DiagnosticTarget::Column(1),
        ];

        assert_eq!(cycle_diagnostic(&targets, None, false), Some(targets[0]));
        assert_eq!(
            cycle_diagnostic(&targets, Some(targets[1]), false),
            Some(targets[0])
        );
        assert_eq!(cycle_diagnostic(&targets, None, true), Some(targets[1]));
        assert_eq!(
            cycle_diagnostic(&targets, Some(targets[0]), true),
            Some(targets[1])
        );
    }
}
