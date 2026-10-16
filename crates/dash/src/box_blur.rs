#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxBlur {
    width: usize,
    height: usize,
    radius: usize,
    passes: usize,
}

impl BoxBlur {
    #[must_use]
    pub const fn new(width: usize, height: usize, radius: usize, passes: usize) -> Self {
        Self { width, height, radius, passes }
    }

    #[must_use]
    pub fn blur(&self, values: &[f32]) -> Vec<f32> {
        let mut out = values.to_vec();
        for _ in 0..self.passes {
            out = self.blur_columns(&self.blur_rows(&out));
        }
        out
    }

    fn blur_rows(&self, values: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; values.len()];
        for row in 0..self.height {
            let span = row * self.width..(row + 1) * self.width;
            self.blur_line(&values[span.clone()], &mut out[span]);
        }
        out
    }

    fn blur_columns(&self, values: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; values.len()];
        let mut line = vec![0.0; self.height];
        let mut blurred = vec![0.0; self.height];
        for column in 0..self.width {
            for row in 0..self.height {
                line[row] = values[row * self.width + column];
            }
            self.blur_line(&line, &mut blurred);
            for row in 0..self.height {
                out[row * self.width + column] = blurred[row];
            }
        }
        out
    }

    fn blur_line(&self, line: &[f32], out: &mut [f32]) {
        let length = line.len();
        if length == 0 {
            return;
        }
        let at = |index: isize| line[index.clamp(0, length as isize - 1) as usize];
        let span = (2 * self.radius + 1) as f32;
        let radius = self.radius as isize;
        let mut sum: f32 = (-radius..=radius).map(at).sum();
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = sum / span;
            let index = index as isize;
            sum += at(index + radius + 1) - at(index - radius);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BoxBlur;

    #[test]
    fn a_flat_field_stays_flat() {
        let blurred = BoxBlur::new(8, 6, 2, 2).blur(&[0.5; 48]);

        assert!(blurred.iter().all(|value| (value - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_single_bright_pixel_spreads_to_its_neighbours_and_keeps_its_total() {
        let mut values = vec![0.0; 21 * 21];
        values[10 * 21 + 10] = 1.0;
        let blurred = BoxBlur::new(21, 21, 1, 1).blur(&values);

        assert!(blurred[10 * 21 + 11] > 0.0 && blurred[11 * 21 + 10] > 0.0);
        assert!(blurred[10 * 21 + 13].abs() < 1e-9);
        assert!((blurred.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
