use crate::{FumeJar, HexColor, ValueNoise};

#[derive(Debug, Clone, PartialEq)]
pub struct GasCloud {
    pub left: f64,
    pub top: f64,
    pub cell: f64,
    pub columns: usize,
    pub rows: usize,
    density: Vec<f64>,
    pub colour: HexColor,
}

impl GasCloud {
    const CELL: f64 = 4.0;
    const DITHER: [f64; 16] = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];
    pub const MOST: f64 = 0.82;
    const WALL: f64 = -0.03;
    const SOFT_WALL: f64 = 0.03;
    const FOOT: (f64, f64) = (0.1, 0.2);
    const MOUTH: (f64, f64) = (0.92, 0.72);
    const LAYERING: f64 = 0.3;
    const SWIRL: f64 = 0.15;
    const SWIRL_SCALE: f64 = 2.6;
    const SWIRL_SPEED: (f64, f64) = (0.11, 0.07);
    const FRONT: f64 = 0.25;
    const SUCK: f64 = 1.4;

    #[must_use]
    pub fn of(jar: &FumeJar, now: f64) -> Self {
        let cell = Self::CELL;
        let (columns, rows) = ((jar.width / cell).ceil() as usize + 1, (jar.height / cell).ceil() as usize + 1);
        let (left, top) = (jar.left(), jar.top());
        let noise = ValueNoise::new(0x9a5 + jar.fate.jar().unwrap_or(0) as u64);
        let (emptied, pull) = (jar.emptied(), jar.pull());
        let front = emptied * (1.0 + Self::FRONT) - Self::FRONT;
        let mut density = vec![0.0; columns * rows];
        for row in 0..rows {
            let rise = row as f64 * cell / jar.height;
            let room = jar.reach(rise) - Self::WALL;
            if room <= 0.0 || rise > 1.0 {
                continue;
            }
            let layer = Self::ramp(rise, Self::FOOT) * Self::ramp(rise, Self::MOUTH) * (1.0 - Self::LAYERING * rise) * Self::ramp(rise, (front, front + Self::FRONT));
            let sink = pull * Self::SUCK * (1.0 - rise);
            for column in 0..columns {
                let across = (column as f64 * cell - jar.width * 0.5) / (jar.width * 0.5);
                let wall = Self::ramp((room - across.abs()) / room, (0.0, Self::SOFT_WALL));
                let drift = noise.fractal(across * Self::SWIRL_SCALE * (1.0 + sink) + now * Self::SWIRL_SPEED.0, rise * Self::SWIRL_SCALE * 1.8 - now * (Self::SWIRL_SPEED.1 + sink), 3);
                let swirl = Self::SWIRL * (1.0 + pull * 2.0);
                density[row * columns + column] = layer * wall * (1.0 - swirl + swirl * drift);
            }
        }
        let thickness = Self::MOST * jar.charge();
        density.iter_mut().for_each(|value| *value = (*value * thickness).clamp(0.0, 1.0));
        Self { left, top, cell, columns, rows, density, colour: jar.fate.colour() }
    }

    fn ramp(value: f64, (from, to): (f64, f64)) -> f64 {
        let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    #[must_use]
    pub fn opacity(&self, column: usize, row: usize) -> f64 {
        if column >= self.columns {
            return 0.0;
        }
        self.density.get(row * self.columns + column).copied().unwrap_or(0.0)
    }

    #[must_use]
    pub fn density_at(&self, x: f64, y: f64) -> f64 {
        let (column, row) = ((x - self.left) / self.cell, (y - self.top) / self.cell);
        if column < 0.0 || row < 0.0 {
            return 0.0;
        }
        let (left, top) = (column.floor() as usize, row.floor() as usize);
        let (across, down) = (column.fract(), row.fract());
        let upper = self.opacity(left, top) * (1.0 - across) + self.opacity(left + 1, top) * across;
        let lower = self.opacity(left, top + 1) * (1.0 - across) + self.opacity(left + 1, top + 1) * across;
        upper * (1.0 - down) + lower * down
    }

    #[must_use]
    pub fn centre(&self, column: usize, row: usize) -> (f64, f64) {
        (self.left + column as f64 * self.cell, self.top + row as f64 * self.cell)
    }

    #[must_use]
    pub fn pixels(&self) -> Vec<u8> {
        let ink = self.colour.channels().map(f64::from);
        self.density
            .iter()
            .enumerate()
            .flat_map(|(index, opacity)| {
                let offset = Self::DITHER[(index / self.columns % 4) * 4 + index % self.columns % 4] / 16.0;
                let [red, green, blue] = ink.map(|channel| (channel * opacity + offset).floor().clamp(0.0, 255.0) as u8);
                [red, green, blue, 255]
            })
            .collect()
    }

    #[must_use]
    pub fn image_bounds(&self) -> (f64, f64, f64, f64) {
        (self.left - self.cell * 0.5, self.top - self.cell * 0.5, self.columns as f64 * self.cell, self.rows as f64 * self.cell)
    }
}

#[cfg(test)]
mod tests {
    use super::GasCloud;
    use crate::{FumeFate, FumeJar};

    fn jar(fumes: usize) -> FumeJar {
        let mut jar = FumeJar::new(FumeFate::Sampled, 100.0, 0.0, 400.0, 223.0);
        for _ in 0..fumes {
            jar.collect();
        }
        for _ in 0..1200 {
            jar.advance(1.0 / 60.0);
        }
        jar
    }

    fn opacities(cloud: &GasCloud, rises: (f64, f64), inner: f64, jar: &FumeJar) -> Vec<f64> {
        let mut found = Vec::new();
        for row in 0..cloud.rows {
            for column in 0..cloud.columns {
                let (x, y) = cloud.centre(column, row);
                let rise = (y - jar.top()) / jar.height;
                let across = (x - jar.centre) / (jar.width * 0.5);
                if (rises.0..rises.1).contains(&rise) && across.abs() < jar.reach(rise) * inner {
                    found.push(cloud.opacity(column, row));
                }
            }
        }
        found
    }

    #[test]
    fn an_empty_jar_holds_no_gas() {
        let jar = jar(0);
        let cloud = GasCloud::of(&jar, 3.0);

        assert!((0..cloud.rows).all(|row| (0..cloud.columns).all(|column| cloud.opacity(column, row) == 0.0)));
    }

    #[test]
    fn the_gas_is_one_even_cloud_through_the_belly() {
        let jar = jar(30);
        for now in [0.0, 7.5, 40.0] {
            let cloud = GasCloud::of(&jar, now);
            let belly = opacities(&cloud, (0.3, 0.55), 0.6, &jar);
            let (low, high) = belly.iter().fold((f64::INFINITY, 0.0_f64), |(low, high), value| (low.min(*value), high.max(*value)));

            assert!(belly.len() > 50);
            assert!(low > high * 0.6, "t={now}: {low} .. {high}");
        }
    }

    #[test]
    fn more_fumes_thicken_the_whole_cloud() {
        let (few, many) = (jar(4), jar(120));
        let mean = |jar: &FumeJar| {
            let found = opacities(&GasCloud::of(jar, 1.0), (0.3, 0.55), 0.6, jar);
            found.iter().sum::<f64>() / found.len() as f64
        };

        assert!(mean(&few) > 0.0);
        assert!(mean(&many) > mean(&few) * 3.0);
        assert!(mean(&many) <= GasCloud::MOST);
    }

    #[test]
    fn the_cloud_becomes_one_opaque_pixel_per_cell_with_density_scaling_its_colour() {
        let jar = jar(60);
        let cloud = GasCloud::of(&jar, 1.0);
        let pixels = cloud.pixels();
        let (column, row) = (cloud.columns / 2, cloud.rows * 2 / 5);
        let at = (row * cloud.columns + column) * 4;
        let (left, top, width, height) = cloud.image_bounds();
        let opacity = cloud.opacity(column, row);

        assert_eq!(pixels.len(), cloud.columns * cloud.rows * 4);
        assert!(opacity > 0.25);
        for (channel, ink) in cloud.colour.channels().iter().enumerate() {
            assert!((f64::from(pixels[at + channel]) - f64::from(*ink) * opacity).abs() <= 1.0);
        }
        assert_eq!(pixels[at + 3], 255);
        assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
        assert!((left + cloud.cell * 0.5 - cloud.centre(0, 0).0).abs() < 1e-9 && (top + cloud.cell * 0.5 - cloud.centre(0, 0).1).abs() < 1e-9);
        assert!((width - cloud.columns as f64 * cloud.cell).abs() < 1e-9 && (height - cloud.rows as f64 * cloud.cell).abs() < 1e-9);
    }

    #[test]
    fn the_gas_reaches_past_the_profile_only_by_the_sliver_the_mask_trims_and_gathers_under_the_foot() {
        let jar = jar(60);
        let cloud = GasCloud::of(&jar, 2.0);
        let mean = |rises: (f64, f64)| {
            let found = opacities(&cloud, rises, 0.5, &jar);
            found.iter().sum::<f64>() / found.len() as f64
        };

        for row in 0..cloud.rows {
            for column in 0..cloud.columns {
                let (x, y) = cloud.centre(column, row);
                let rise = (y - jar.top()) / jar.height;
                let across = ((x - jar.centre) / (jar.width * 0.5)).abs();
                assert!(across <= jar.reach(rise) + 0.04 || cloud.opacity(column, row) < 1e-9, "{x} {y}");
            }
        }
        assert!(mean((0.25, 0.4)) > mean((0.6, 0.75)));
    }

    #[test]
    fn the_gas_fills_the_belly_out_close_to_the_glass() {
        let jar = jar(60);
        let cloud = GasCloud::of(&jar, 2.0);
        let middle = opacities(&cloud, (0.33, 0.42), 0.3, &jar);
        let mean = |found: &[f64]| found.iter().sum::<f64>() / found.len() as f64;
        let outer: Vec<f64> = {
            let mut found = Vec::new();
            for row in 0..cloud.rows {
                for column in 0..cloud.columns {
                    let (x, y) = cloud.centre(column, row);
                    let rise = (y - jar.top()) / jar.height;
                    let across = ((x - jar.centre) / (jar.width * 0.5)).abs() / jar.reach(rise);
                    if (0.33..0.42).contains(&rise) && (0.86..0.9).contains(&across) {
                        found.push(cloud.opacity(column, row));
                    }
                }
            }
            found
        };

        assert!(!outer.is_empty());
        assert!(mean(&outer) > mean(&middle) * 0.7, "{} vs {}", mean(&outer), mean(&middle));
    }

    #[test]
    fn the_density_between_cells_blends_its_four_neighbours() {
        let cloud = GasCloud::of(&jar(60), 1.0);
        let (column, row) = (cloud.columns / 2, cloud.rows * 2 / 5);
        let (x, y) = cloud.centre(column, row);
        let midway = cloud.density_at(x + cloud.cell * 0.5, y);

        assert!((cloud.density_at(x, y) - cloud.opacity(column, row)).abs() < 1e-9);
        assert!((midway - (cloud.opacity(column, row) + cloud.opacity(column + 1, row)) * 0.5).abs() < 1e-9);
        assert_eq!(cloud.density_at(cloud.left - 1.0, y), 0.0);
    }

    #[test]
    fn the_cloud_is_fine_enough_that_stretching_it_shows_no_cells() {
        for width in [120.0, 223.0, 400.0] {
            let jar = FumeJar::new(FumeFate::Sampled, 300.0, 0.0, width / 0.563, width);

            assert!(GasCloud::of(&jar, 0.0).cell <= 4.0);
        }
    }

    #[test]
    fn thin_gas_keeps_a_smooth_ramp_after_rounding() {
        let mut cloud = GasCloud::of(&jar(60), 1.0);
        let steps = 64;
        cloud.columns = steps;
        cloud.rows = 4;
        cloud.density = (0..cloud.rows).flat_map(|_| (0..steps).map(|step| 0.12 * step as f64 / steps as f64)).collect();
        let pixels = cloud.pixels();
        let green = |column: usize| (0..4).map(|row| f64::from(pixels[(row * steps + column) * 4 + 1])).sum::<f64>() / 4.0;
        let ink = f64::from(cloud.colour.channels()[1]);

        for column in 0..steps {
            let wanted = ink * 0.12 * column as f64 / steps as f64;
            assert!((green(column) - wanted).abs() < 0.75, "{column}: {} vs {wanted}", green(column));
        }
    }

    #[test]
    fn a_draining_jar_clears_from_the_foot_down_to_the_mouth() {
        let mut jar = jar(60);
        let drain = jar.harvest(0.0).expect("drains");
        for _ in 0..((drain.seconds * 0.5) * 60.0) as usize {
            jar.advance(1.0 / 60.0);
        }
        let cloud = GasCloud::of(&jar, 1.0);
        let mean = |rises: (f64, f64)| {
            let found = opacities(&cloud, rises, 0.5, &jar);
            found.iter().sum::<f64>() / found.len() as f64
        };

        assert!(mean((0.15, 0.3)) < 0.02, "{}", mean((0.15, 0.3)));
        assert!(mean((0.5, 0.62)) > 0.15, "{}", mean((0.5, 0.62)));
        for _ in 0..((drain.seconds * 0.6) * 60.0) as usize {
            jar.advance(1.0 / 60.0);
        }
        let empty = GasCloud::of(&jar, 2.0);
        assert!((0..empty.rows).all(|row| (0..empty.columns).all(|column| empty.opacity(column, row) < 1e-9)));
    }

    #[test]
    fn the_cloud_moves_slowly() {
        let jar = jar(60);
        let (now, soon) = (GasCloud::of(&jar, 10.0), GasCloud::of(&jar, 10.0 + 1.0 / 60.0));
        let later = GasCloud::of(&jar, 25.0);
        let change = |other: &GasCloud| {
            (0..now.rows).flat_map(|row| (0..now.columns).map(move |column| (column, row))).map(|(column, row)| (now.opacity(column, row) - other.opacity(column, row)).abs()).fold(0.0, f64::max)
        };

        assert!(change(&soon) < 0.01);
        assert!(change(&later) > 0.02);
    }
}
