use std::f64::consts::TAU;
use std::fmt::Write;

use crate::SplitMix;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChalkStroke {
    seed: u64,
    wobble: f64,
}

impl ChalkStroke {
    const STEP: f64 = 9.0;
    const SMOOTHING: f64 = 0.7;
    const PASS_SEED: u64 = 0x5DEE_CE66_D1CE_4E5B;
    const LOOP_SEGMENTS: usize = 44;
    const LOOP_TURNS: f64 = 1.14;
    const LOOP_SQUASH: f64 = 0.86;

    #[must_use]
    pub const fn new(seed: u64, wobble: f64) -> Self {
        Self { seed, wobble }
    }

    #[must_use]
    pub fn vertices(&self, points: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let mut random = SplitMix::new(self.seed);
        let mut drift = 0.0_f64;
        let Some(&first) = points.first() else {
            return Vec::new();
        };
        let mut drawn = vec![first];
        for pair in points.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let length = dx.hypot(dy);
            if length < f64::EPSILON {
                continue;
            }
            let (nx, ny) = (-dy / length, dx / length);
            let pieces = (length / Self::STEP).ceil().max(1.0) as usize;
            for piece in 1..=pieces {
                let along = piece as f64 / pieces as f64;
                drift = (Self::SMOOTHING * drift + (1.0 - Self::SMOOTHING) * random.gaussian() * self.wobble)
                    .clamp(-self.wobble, self.wobble);
                drawn.push((x0 + dx * along + nx * drift, y0 + dy * along + ny * drift));
            }
        }
        drawn
    }

    #[must_use]
    pub fn trace(&self, points: &[(f64, f64)]) -> String {
        Self::path(&self.vertices(points), false)
    }

    #[must_use]
    pub fn passes(&self, points: &[(f64, f64)], count: u64) -> Vec<String> {
        (0..count)
            .map(|pass| Self::new(self.seed.wrapping_add(pass.wrapping_mul(Self::PASS_SEED)), self.wobble).trace(points))
            .collect()
    }

    #[must_use]
    pub fn loop_vertices(&self, centre: (f64, f64), radius: f64) -> Vec<(f64, f64)> {
        let mut random = SplitMix::new(self.seed);
        let start = random.unit() * TAU;
        let mut drift = 0.0_f64;
        (0..=Self::LOOP_SEGMENTS)
            .map(|segment| {
                let angle = start + TAU * Self::LOOP_TURNS * segment as f64 / Self::LOOP_SEGMENTS as f64;
                drift = (Self::SMOOTHING * drift + (1.0 - Self::SMOOTHING) * random.gaussian() * self.wobble)
                    .clamp(-self.wobble, self.wobble);
                let reach = radius + drift;
                (centre.0 + reach * angle.cos(), centre.1 + reach * Self::LOOP_SQUASH * angle.sin())
            })
            .collect()
    }

    #[must_use]
    pub fn loop_around(&self, centre: (f64, f64), radius: f64) -> String {
        Self::path(&self.loop_vertices(centre, radius), false)
    }

    fn path(vertices: &[(f64, f64)], close: bool) -> String {
        let mut path = String::with_capacity(vertices.len() * 14);
        for (index, (x, y)) in vertices.iter().enumerate() {
            let command = if index == 0 { 'M' } else { 'L' };
            let _ = write!(path, "{command}{x:.1},{y:.1}");
        }
        if close && !path.is_empty() {
            path.push('Z');
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::ChalkStroke;

    #[test]
    fn no_points_draw_nothing() {
        let stroke = ChalkStroke::new(1, 1.0);

        assert!(stroke.vertices(&[]).is_empty());
        assert!(stroke.trace(&[]).is_empty());
    }

    #[test]
    fn the_stroke_starts_on_the_first_point_and_wanders_at_most_the_wobble() {
        let stroke = ChalkStroke::new(7, 1.5);
        let vertices = stroke.vertices(&[(0.0, 50.0), (300.0, 50.0)]);

        assert_eq!(vertices[0], (0.0, 50.0));
        assert!(vertices.len() > 30);
        assert!(vertices.iter().all(|(_, y)| (y - 50.0).abs() <= 1.5 + 1e-9));
        assert!(vertices.iter().any(|(_, y)| (y - 50.0).abs() > 0.05));
    }

    #[test]
    fn the_stroke_ends_near_the_last_point() {
        let vertices = ChalkStroke::new(3, 1.0).vertices(&[(0.0, 0.0), (90.0, 40.0), (200.0, 10.0)]);
        let (x, y) = *vertices.last().unwrap();

        assert!((x - 200.0).hypot(y - 10.0) <= 1.0 + 1e-9);
    }

    #[test]
    fn repeated_points_do_not_produce_nan() {
        let path = ChalkStroke::new(2, 1.0).trace(&[(5.0, 5.0), (5.0, 5.0), (20.0, 5.0)]);

        assert!(path.starts_with('M'));
        assert!(!path.contains("NaN"));
    }

    #[test]
    fn the_same_seed_draws_the_same_stroke_and_passes_differ() {
        let points = [(0.0, 0.0), (100.0, 30.0), (200.0, 0.0)];
        let stroke = ChalkStroke::new(11, 1.2);
        let passes = stroke.passes(&points, 3);

        assert_eq!(stroke.trace(&points), ChalkStroke::new(11, 1.2).trace(&points));
        assert_eq!(passes.len(), 3);
        assert_eq!(passes[0], stroke.trace(&points));
        assert_ne!(passes[0], passes[1]);
        assert_ne!(passes[1], passes[2]);
    }

    #[test]
    fn a_loop_goes_more_than_once_around_its_centre_at_about_its_radius() {
        let vertices = ChalkStroke::new(5, 1.0).loop_vertices((50.0, 50.0), 10.0);
        let angles: Vec<f64> = vertices.iter().map(|(x, y)| (y - 50.0).atan2(x - 50.0)).collect();
        let swept: f64 = angles
            .windows(2)
            .map(|pair| {
                let step = pair[1] - pair[0];
                (step + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
            })
            .sum();

        assert!(swept.abs() > std::f64::consts::TAU);
        assert!(vertices.iter().all(|(x, y)| {
            let reach = (x - 50.0).hypot((y - 50.0) / 0.86);
            (8.9..=11.1).contains(&reach)
        }));
    }
}
