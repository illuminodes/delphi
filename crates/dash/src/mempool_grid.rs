/// One transaction's square: its index in the input order, its top-left cell and its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridTile {
    pub index: usize,
    pub x: u32,
    pub y: u32,
    pub side: u32,
}

/// Transactions packed as squares into a grid of fixed size, in order, each placed at the
/// first free spot (top to bottom, left to right) where it fits. Small squares fill the gaps
/// left beside large ones, as on mempool.space. Packing stops at the first square with no room
/// left, so the grid holds the newest transactions and nothing older.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolGrid {
    columns: u32,
    rows: u32,
    tiles: Vec<GridTile>,
}

impl MempoolGrid {
    /// The largest side, for transactions worth $100,000 or more.
    pub const MAX_SIDE: u32 = 5;

    /// The side of a transaction's square from its dollar value: one cell under $100, then one
    /// more per tenfold. Without a value it gets the smallest square.
    #[must_use]
    pub fn side_for(usd: Option<f64>) -> u32 {
        let Some(usd) = usd.filter(|usd| usd.is_finite() && *usd >= 100.0) else {
            return 1;
        };
        let decades = usd.log10().floor() as u32;
        decades.saturating_sub(1).clamp(1, Self::MAX_SIDE)
    }

    #[must_use]
    pub fn pack(sides: &[u32], columns: u32, rows: u32) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        let mut taken = vec![vec![false; columns as usize]; rows as usize];
        let mut first_open = 0;
        let mut tiles = Vec::with_capacity(sides.len());
        for (index, side) in sides.iter().enumerate() {
            let side = (*side).clamp(1, columns.min(rows));
            let Some((x, y)) = Self::find(&taken, &mut first_open, side) else {
                break;
            };
            for row in &mut taken[y as usize..(y + side) as usize] {
                row[x as usize..(x + side) as usize].fill(true);
            }
            tiles.push(GridTile { index, x, y, side });
        }
        Self { columns, rows, tiles }
    }

    fn find(taken: &[Vec<bool>], first_open: &mut usize, side: u32) -> Option<(u32, u32)> {
        while taken.get(*first_open).is_some_and(|row| row.iter().all(|cell| *cell)) {
            *first_open += 1;
        }
        let side = side as usize;
        (*first_open..=taken.len().checked_sub(side)?).find_map(|y| {
            let band = &taken[y..y + side];
            (0..=band[0].len() - side)
                .find(|x| band.iter().all(|row| row[*x..x + side].iter().all(|cell| !cell)))
                .map(|x| (x as u32, y as u32))
        })
    }

    #[must_use]
    pub const fn columns(&self) -> u32 {
        self.columns
    }

    #[must_use]
    pub const fn rows(&self) -> u32 {
        self.rows
    }

    #[must_use]
    pub fn tiles(&self) -> &[GridTile] {
        &self.tiles
    }
}

#[cfg(test)]
mod tests {
    use super::MempoolGrid;

    #[test]
    fn the_side_grows_by_one_cell_per_tenfold_in_value() {
        let sides: Vec<u32> = [None, Some(5.0), Some(99.0), Some(100.0), Some(5_000.0), Some(20_000.0), Some(1e9)]
            .into_iter()
            .map(MempoolGrid::side_for)
            .collect();

        assert_eq!(sides, vec![1, 1, 1, 1, 2, 3, MempoolGrid::MAX_SIDE]);
    }

    #[test]
    fn squares_are_placed_in_order_and_small_ones_fill_the_gap_beside_a_large_one() {
        let grid = MempoolGrid::pack(&[2, 1, 1, 1, 1], 3, 3);
        let places: Vec<(u32, u32)> = grid.tiles().iter().map(|tile| (tile.x, tile.y)).collect();

        assert_eq!(places, vec![(0, 0), (2, 0), (2, 1), (0, 2), (1, 2)]);
    }

    #[test]
    fn packing_stops_at_the_first_square_without_room_so_nothing_older_slips_in() {
        let grid = MempoolGrid::pack(&[2, 2, 2, 1, 1], 4, 3);
        let indices: Vec<usize> = grid.tiles().iter().map(|tile| tile.index).collect();

        assert_eq!(indices, vec![0, 1]);
        assert!(grid.tiles().iter().all(|tile| tile.y + tile.side <= grid.rows()));
    }

    #[test]
    fn no_two_squares_overlap_and_all_stay_inside_the_columns() {
        let sides: Vec<u32> = (0..200).map(|index| [1, 3, 1, 2, 5, 1, 1, 4][index % 8]).collect();
        let grid = MempoolGrid::pack(&sides, 12, 400);
        let mut cells = vec![vec![false; 12]; grid.rows() as usize];

        for tile in grid.tiles() {
            assert!(tile.x + tile.side <= grid.columns());
            for row in &mut cells[tile.y as usize..(tile.y + tile.side) as usize] {
                for cell in &mut row[tile.x as usize..(tile.x + tile.side) as usize] {
                    assert!(!*cell, "overlap at tile {}", tile.index);
                    *cell = true;
                }
            }
        }
        assert_eq!(grid.tiles().len(), 200);
    }

    #[test]
    fn a_square_wider_than_the_grid_is_shrunk_to_fit() {
        let grid = MempoolGrid::pack(&[5, 5], 2, 3);

        assert_eq!(grid.tiles()[0].side, 2);
        assert_eq!(grid.tiles().len(), 1);
    }
}
