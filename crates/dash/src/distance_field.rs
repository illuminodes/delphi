#[derive(Debug, Clone, PartialEq)]
pub struct DistanceField {
    width: usize,
    height: usize,
    squared: Vec<f64>,
}

impl DistanceField {
    const FAR: f64 = 1e20;

    #[must_use]
    pub fn new(width: usize, height: usize, inside: impl Fn(usize) -> bool) -> Self {
        let mut squared: Vec<f64> = (0..width * height).map(|index| if inside(index) { Self::FAR } else { 0.0 }).collect();
        let mut line = vec![0.0; width.max(height)];
        for column in 0..width {
            for row in 0..height {
                line[row] = squared[row * width + column];
            }
            let swept = Self::sweep(&line[..height]);
            for (row, value) in swept.into_iter().enumerate() {
                squared[row * width + column] = value;
            }
        }
        for row in 0..height {
            let swept = Self::sweep(&squared[row * width..(row + 1) * width]);
            squared[row * width..(row + 1) * width].copy_from_slice(&swept);
        }
        Self { width, height, squared }
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize) -> f64 {
        self.squared.get(row * self.width + column).map_or(0.0, |squared| squared.sqrt())
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    fn sweep(costs: &[f64]) -> Vec<f64> {
        let count = costs.len();
        let mut swept = vec![0.0; count];
        if count == 0 {
            return swept;
        }
        let mut hulls = vec![0usize; count];
        let mut bounds = vec![0.0; count + 1];
        let mut top = 0;
        bounds[0] = f64::NEG_INFINITY;
        bounds[1] = f64::INFINITY;
        let parabola = |at: usize| costs[at] + (at * at) as f64;
        for position in 1..count {
            let mut crossing;
            loop {
                let hull = hulls[top];
                crossing = (parabola(position) - parabola(hull)) / (2.0 * (position - hull) as f64);
                if crossing <= bounds[top] && top > 0 {
                    top -= 1;
                } else {
                    break;
                }
            }
            if crossing <= bounds[top] {
                hulls[0] = position;
                bounds[1] = f64::INFINITY;
                continue;
            }
            top += 1;
            hulls[top] = position;
            bounds[top] = crossing;
            bounds[top + 1] = f64::INFINITY;
        }
        let mut hull = 0;
        for (position, value) in swept.iter_mut().enumerate() {
            while bounds[hull + 1] < position as f64 {
                hull += 1;
            }
            let offset = position as f64 - hulls[hull] as f64;
            *value = offset * offset + costs[hulls[hull]];
        }
        swept
    }
}

#[cfg(test)]
mod tests {
    use super::DistanceField;

    #[test]
    fn every_pixel_knows_its_straight_line_distance_to_the_nearest_outside_pixel() {
        let field = DistanceField::new(21, 15, |index| index != 7 * 21 + 10);

        for (column, row) in [(10, 7), (13, 11), (0, 0), (20, 14), (10, 2)] {
            let expected = (column as f64 - 10.0).hypot(row as f64 - 7.0);
            assert!((field.at(column, row) - expected).abs() < 1e-9, "({column}, {row}): {} vs {expected}", field.at(column, row));
        }
    }

    #[test]
    fn a_band_is_deepest_along_its_middle() {
        let field = DistanceField::new(10, 11, |index| (2..9).contains(&(index / 10)));

        assert!((field.at(4, 5) - 4.0).abs() < 1e-9);
        assert!((field.at(4, 3) - 2.0).abs() < 1e-9);
        assert!(field.at(4, 0).abs() < 1e-9);
    }
}
