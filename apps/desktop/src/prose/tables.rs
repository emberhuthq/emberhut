//! A table that reads at any width. When every column gets enough room to read, a grid whose
//! columns share the width by how much they hold; when they don't (three long columns in a
//! narrow window), each row becomes a small card: its first cell the title, then
//! `column: value` lines. Never a column squeezed to a sliver. A table is measured once, when
//! its message is read ([`Table::new`]); only its layout follows the width.

use super::{copy, links};

/// The most columns of a table drawn; the rest are left out.
pub(crate) const MOST_COLUMNS: usize = 8;

/// The most rows of a table drawn, its header among them; the rest are counted.
pub(crate) const MOST_ROWS: usize = 40;

/// The least a column of words gets in the grid, in pixels.
pub(crate) const NARROWEST: f32 = 96.;

/// About how wide a character of the chat's text is, in pixels (14 px type).
const CHARACTER: f32 = 7.2;

/// A cell's padding, both sides together, in pixels.
const CELL_PADDING: f32 = 16.;

/// How a table is drawn in the width it has.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// A grid, each column this many pixels wide.
    Grid(Vec<f32>),
    /// A card per row.
    Cards,
}

/// A table as the chat draws it: its first rows and columns, squared, measured once.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Table {
    /// The rows drawn, each with as many cells (short rows filled with blanks).
    pub(crate) rows: Vec<Vec<String>>,
    /// How many rows are left out ("… 3 more rows").
    pub(crate) more_rows: usize,
    /// The rows as cards, for a width too narrow for the grid.
    pub(crate) cards: Vec<Card>,
    /// How wide each column wants to be, in pixels.
    wanted: Vec<f32>,
}

impl Table {
    /// `rows` read as a table of at most [`MOST_COLUMNS`] columns, and at most [`MOST_ROWS`]
    /// rows or `cells` cells, whichever is fewer; the cells drawn get their links
    /// ([`links::linked`]).
    #[must_use]
    pub(crate) fn new(rows: &[Vec<String>], cells: usize) -> Self {
        let columns = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .min(MOST_COLUMNS);
        let shown = rows
            .len()
            .min(MOST_ROWS)
            .min(cells.checked_div(columns).unwrap_or(0));
        let more_rows = rows.len() - shown;
        let rows = squared(&rows[..shown], columns);
        Self {
            cards: cards(&rows),
            wanted: wanted(&rows),
            more_rows,
            rows,
        }
    }

    /// How many cells it draws.
    #[must_use]
    pub(crate) fn cells(&self) -> usize {
        self.rows.iter().map(Vec::len).sum()
    }

    /// How it is drawn in `width` pixels: see [`layout`].
    #[must_use]
    pub(crate) fn layout(&self, width: f32) -> Layout {
        layout(&self.wanted, width)
    }
}

/// The rows with `columns` linked cells each (short rows filled with blanks, long ones cut).
fn squared(rows: &[Vec<String>], columns: usize) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| {
            let mut row: Vec<String> = row
                .iter()
                .take(columns)
                .map(|cell| links::linked(cell))
                .collect();
            row.resize(columns, String::new());
            row
        })
        .collect()
}

/// How wide each column of `rows` (squared) wants to be: its longest line, and padding.
fn wanted(rows: &[Vec<String>]) -> Vec<f32> {
    let columns = rows.first().map_or(0, Vec::len);
    (0..columns)
        .map(|column| {
            let longest = rows
                .iter()
                .filter_map(|row| row.get(column))
                .flat_map(|cell| cell.lines())
                .map(|line| copy::preview(line).chars().count())
                .max()
                .unwrap_or(0);
            #[expect(
                clippy::cast_precision_loss,
                reason = "a cell's length in characters, far below f32's exact range"
            )]
            let longest = longest as f32;
            longest.mul_add(CHARACTER, CELL_PADDING)
        })
        .collect()
}

/// How columns that want `wanted` pixels are drawn in `width` pixels. When they don't all fit,
/// each gets the least of what it wants and [`NARROWEST`], and the room left goes to the
/// columns by how much more they would take. When even that doesn't fit: cards.
#[must_use]
fn layout(wanted: &[f32], width: f32) -> Layout {
    let least: Vec<f32> = wanted.iter().map(|&want| want.min(NARROWEST)).collect();
    let (wanted_total, least_total): (f32, f32) = (wanted.iter().sum(), least.iter().sum());
    if wanted.is_empty() || least_total > width {
        return Layout::Cards;
    }
    if wanted_total <= width {
        return Layout::Grid(wanted.to_vec());
    }
    let room = width - least_total;
    let more = wanted_total - least_total;
    Layout::Grid(
        wanted
            .iter()
            .zip(&least)
            .map(|(&want, &least)| least + room * (want - least) / more)
            .collect(),
    )
}

/// One row as a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    /// Its first cell, when it has one.
    pub(crate) title: Option<String>,
    /// Its other cells that aren't blank, each with its column's header (plain words) when the
    /// table has a header.
    pub(crate) lines: Vec<(Option<String>, String)>,
}

/// The rows (squared) as cards: the header names each line; a table of one row is its own.
fn cards(rows: &[Vec<String>]) -> Vec<Card> {
    let head = rows.first().cloned().unwrap_or_default();
    let body = if rows.len() > 1 { &rows[1..] } else { rows };
    body.iter()
        .map(|row| Card {
            title: row.first().filter(|cell| !cell.trim().is_empty()).cloned(),
            lines: row
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, cell)| !cell.trim().is_empty())
                .map(|(column, cell)| {
                    let label = (rows.len() > 1)
                        .then(|| head.get(column).map(|h| copy::preview(h)))
                        .flatten()
                        .filter(|label| !label.is_empty());
                    (label, cell.clone())
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|&c| c.to_owned()).collect()
    }

    fn table(rows: &[Vec<String>]) -> Table {
        Table::new(rows, usize::MAX)
    }

    #[test]
    fn short_rows_are_filled() {
        assert_eq!(
            table(&[row(&["a", "b", "c"]), row(&["d"])]).rows,
            [row(&["a", "b", "c"]), row(&["d", "", ""])]
        );
    }

    #[test]
    fn a_huge_table_is_cut_to_what_reads() {
        let wide = vec!["x".to_owned(); 50];
        let rows = vec![wide; 300];
        let cut = table(&rows);
        assert_eq!(cut.rows.len(), MOST_ROWS);
        assert!(cut.rows.iter().all(|row| row.len() == MOST_COLUMNS));
        assert_eq!(cut.more_rows, 300 - MOST_ROWS);
        let few = Table::new(&rows, MOST_COLUMNS * 3);
        assert_eq!((few.rows.len(), few.more_rows, few.cells()), (3, 297, 24));
        let none = Table::new(&rows, 2);
        assert_eq!((none.rows.len(), none.more_rows), (0, 300), "no cells left");
        assert_eq!(none.layout(500.), Layout::Cards);
    }

    #[test]
    fn a_table_that_fits_takes_its_columns_widths() {
        let rows = [row(&["Plan name", "Size"]), row(&["Small", "12"])];
        let Layout::Grid(widths) = table(&rows).layout(800.) else {
            panic!("a grid")
        };
        assert_eq!(widths.len(), 2);
        assert!(widths.iter().sum::<f32>() < 800.);
        assert!(widths[0] > widths[1], "the first column holds more");
    }

    #[test]
    fn a_wide_table_shares_the_width_by_what_each_column_holds() {
        let long = "word ".repeat(30);
        let rows = [row(&["", "A", "B"]), row(&["Price", &long, "short"])];
        let Layout::Grid(widths) = table(&rows).layout(600.) else {
            panic!("a grid")
        };
        assert!((widths.iter().sum::<f32>() - 600.).abs() < 0.5);
        assert!(widths[1] > widths[2] && widths[1] > widths[0]);
        assert!(widths.iter().all(|&w| w >= 20.));
    }

    #[test]
    fn a_table_too_wide_to_read_becomes_cards() {
        let long = "word ".repeat(30);
        let rows = [
            row(&["", "**Mac**", "**Spark**"]),
            row(&["Price", &long, &long]),
            row(&["Noise", "Silent", ""]),
        ];
        assert_eq!(table(&rows).layout(200.), Layout::Cards);
        assert_eq!(
            table(&rows).cards,
            [
                Card {
                    title: Some("Price".into()),
                    lines: vec![
                        (Some("Mac".into()), long.clone()),
                        (Some("Spark".into()), long.clone())
                    ],
                },
                Card {
                    title: Some("Noise".into()),
                    lines: vec![(Some("Mac".into()), "Silent".into())],
                },
            ]
        );
    }

    #[test]
    fn a_table_of_one_row_is_its_own_card() {
        assert_eq!(
            table(&[row(&["Only", "row"])]).cards,
            [Card {
                title: Some("Only".into()),
                lines: vec![(None, "row".into())]
            }]
        );
        assert_eq!(table(&[]).layout(500.), Layout::Cards);
    }
}
