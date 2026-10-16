use crate::CarveRelief;

#[derive(Debug, Clone, PartialEq)]
pub struct ReliefGrid {
    columns: usize,
    rows: usize,
    heights: Vec<f32>,
}

impl ReliefGrid {
    #[must_use]
    pub fn sample(relief: &CarveRelief, size: (f64, f64), budget: usize) -> Self {
        let (width, height) = size;
        let spacing = (width * height / budget.max(1) as f64).sqrt().max(1.0);
        let columns = (width / spacing).floor() as usize + 1;
        let rows = (height / spacing).floor() as usize + 1;
        let (across, down) = (relief.width() as f64 / width, relief.height() as f64 / height);
        let heights = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| {
                let x = column as f64 * width / (columns - 1).max(1) as f64;
                let y = row as f64 * height / (rows - 1).max(1) as f64;
                let pixel = ((x * across) as usize).min(relief.width().saturating_sub(1));
                let line = ((y * down) as usize).min(relief.height().saturating_sub(1));
                relief.at(pixel, line) as f32
            })
            .collect();
        Self { columns, rows, heights }
    }

    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }

    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }
}

#[cfg(test)]
mod tests {
    use super::ReliefGrid;
    use crate::{CarveGroove, CarveMask, CarveRelief};

    struct GridFixture;

    impl GridFixture {
        fn relief() -> CarveRelief {
            let mut mask = CarveMask::new(200, 100, 2.0);
            mask.stamp_groove(&CarveGroove::line((50.0, 5.0), (50.0, 45.0), 12.0));
            CarveRelief::new(&mask, 1.0, 4.0)
        }
    }

    #[test]
    fn a_generous_budget_puts_a_vertex_on_every_css_pixel() {
        let grid = ReliefGrid::sample(&GridFixture::relief(), (100.0, 50.0), 1_000_000);

        assert_eq!((grid.columns(), grid.rows()), (101, 51));
        assert_eq!(grid.heights().len(), 101 * 51);
    }

    #[test]
    fn a_tight_budget_spreads_the_vertices_out() {
        let grid = ReliefGrid::sample(&GridFixture::relief(), (100.0, 50.0), 1250);

        assert!(grid.columns() * grid.rows() <= 1400, "{} x {}", grid.columns(), grid.rows());
        assert!(grid.columns() > 2 * grid.rows() - 4);
    }

    #[test]
    fn the_vertices_follow_the_cut() {
        let grid = ReliefGrid::sample(&GridFixture::relief(), (100.0, 50.0), 1_000_000);
        let at = |column: usize, row: usize| grid.heights()[row * grid.columns() + column];

        assert!(at(50, 25) > 3.5, "{}", at(50, 25));
        assert!(at(10, 25).abs() < 1e-6);
    }
}
