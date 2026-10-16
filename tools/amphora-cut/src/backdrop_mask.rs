use std::collections::VecDeque;

use image::RgbImage;

pub struct BackdropMask {
    pub width: usize,
    pub height: usize,
    backdrop: Vec<bool>,
}

impl BackdropMask {
    pub const BRIGHT: f64 = 96.0;
    const GREY: f64 = 22.0;
    const STEP: f64 = 40.0;
    const POCKET: usize = 4;
    pub const POCKET_SHARE: f64 = 0.004;

    fn pale(pixel: [f64; 3], bright: f64) -> bool {
        let (low, high) = (pixel.iter().copied().fold(f64::INFINITY, f64::min), pixel.iter().copied().fold(0.0, f64::max));
        low > bright && high - low < Self::GREY
    }

    pub fn flood_keeping(photo: &RgbImage, bright: f64, pocket_share: f64) -> Self {
        let (width, height) = (photo.width() as usize, photo.height() as usize);
        let colour = |x: usize, y: usize| photo.get_pixel(x as u32, y as u32).0.map(f64::from);
        let distance = |one: [f64; 3], two: [f64; 3]| ((one[0] - two[0]).powi(2) + (one[1] - two[1]).powi(2) + (one[2] - two[2]).powi(2)).sqrt();
        let border: Vec<(usize, usize)> = (0..width).flat_map(|x| [(x, 0), (x, height - 1)]).chain((0..height).flat_map(|y| [(0, y), (width - 1, y)])).collect();
        let mut backdrop = vec![false; width * height];
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
        let pockets = Self::pockets(photo, width, height, bright);
        for (x, y) in border.into_iter().chain(pockets) {
            if Self::pale(colour(x, y), bright) && !backdrop[y * width + x] {
                backdrop[y * width + x] = true;
                queue.push_back((x, y));
            }
        }
        while let Some((x, y)) = queue.pop_front() {
            let here = colour(x, y);
            let neighbours = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
            for (next_x, next_y) in neighbours {
                if next_x >= width || next_y >= height || backdrop[next_y * width + next_x] {
                    continue;
                }
                let there = colour(next_x, next_y);
                if Self::pale(there, bright) && distance(there, here) < Self::STEP {
                    backdrop[next_y * width + next_x] = true;
                    queue.push_back((next_x, next_y));
                }
            }
        }
        let mut mask = Self { width, height, backdrop };
        mask.keep_largest_piece();
        mask.fill_specks(pocket_share);
        mask
    }

    fn fill_specks(&mut self, pocket_share: f64) {
        let (width, height) = (self.width, self.height);
        let smallest_pocket = (width * height) as f64 * pocket_share;
        let mut seen = vec![false; width * height];
        for start in 0..width * height {
            if !self.backdrop[start] || seen[start] {
                continue;
            }
            let mut members = Vec::new();
            let mut touches_border = false;
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(at) = queue.pop_front() {
                members.push(at);
                let (x, y) = (at % width, at / width);
                touches_border |= x == 0 || y == 0 || x + 1 == width || y + 1 == height;
                for (next_x, next_y) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    if next_x >= width || next_y >= height {
                        continue;
                    }
                    let next = next_y * width + next_x;
                    if self.backdrop[next] && !seen[next] {
                        seen[next] = true;
                        queue.push_back(next);
                    }
                }
            }
            if !touches_border && (members.len() as f64) < smallest_pocket {
                members.into_iter().for_each(|at| self.backdrop[at] = false);
            }
        }
    }

    fn keep_largest_piece(&mut self) {
        let (width, height) = (self.width, self.height);
        let mut piece = vec![usize::MAX; width * height];
        let mut sizes = Vec::new();
        for start in 0..width * height {
            if self.backdrop[start] || piece[start] != usize::MAX {
                continue;
            }
            let label = sizes.len();
            let mut size = 0;
            let mut queue = VecDeque::from([start]);
            piece[start] = label;
            while let Some(at) = queue.pop_front() {
                size += 1;
                let (x, y) = (at % width, at / width);
                for (next_x, next_y) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    let next = next_y * width + next_x;
                    if next_x < width && next_y < height && !self.backdrop[next] && piece[next] == usize::MAX {
                        piece[next] = label;
                        queue.push_back(next);
                    }
                }
            }
            sizes.push(size);
        }
        let Some(largest) = (0..sizes.len()).max_by_key(|label| sizes[*label]) else {
            return;
        };
        for (backdrop, label) in self.backdrop.iter_mut().zip(piece) {
            if label != largest {
                *backdrop = true;
            }
        }
    }

    fn pockets(photo: &RgbImage, width: usize, height: usize, bright: f64) -> Vec<(usize, usize)> {
        let colour = |x: usize, y: usize| photo.get_pixel(x as u32, y as u32).0.map(f64::from);
        let mut seeds = Vec::new();
        for y in 0..height {
            let row: Vec<bool> = (0..width).map(|x| Self::pale(colour(x, y), bright)).collect();
            let dark: Vec<usize> = (0..width).filter(|x| !row[*x]).collect();
            let (Some(first), Some(last)) = (dark.first(), dark.last()) else {
                continue;
            };
            let mut x = *first;
            while x <= *last {
                if row[x] {
                    let start = x;
                    while x <= *last && row[x] {
                        x += 1;
                    }
                    if x - start >= Self::POCKET {
                        seeds.push(((start + x) / 2, y));
                    }
                }
                x += 1;
            }
        }
        seeds
    }

    pub fn vessel(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height && !self.backdrop[y * self.width + x]
    }

    pub fn bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut found: Option<(usize, usize, usize, usize)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.vessel(x, y) {
                    found = Some(found.map_or((x, y, x, y), |(left, top, right, bottom)| (left.min(x), top.min(y), right.max(x), bottom.max(y))));
                }
            }
        }
        found
    }

    pub fn coverage(&self, x: usize, y: usize) -> f64 {
        let mut inside = 0;
        for down in -1_i64..=1 {
            for across in -1_i64..=1 {
                let (at_x, at_y) = (x as i64 + across, y as i64 + down);
                if at_x >= 0 && at_y >= 0 && self.vessel(at_x as usize, at_y as usize) {
                    inside += 1;
                }
            }
        }
        f64::from(inside) / 9.0
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::BackdropMask;

    fn ring_with_a_window() -> RgbImage {
        RgbImage::from_fn(200, 200, |x, y| {
            let inside_ring = (40..160).contains(&x) && (40..160).contains(&y);
            let window = (95..105).contains(&x) && (95..105).contains(&y);
            if inside_ring && !window { Rgb([60, 70, 50]) } else { Rgb([230, 230, 226]) }
        })
    }

    #[test]
    fn a_small_enclosed_window_is_filled_by_default_and_kept_open_when_asked() {
        let photo = ring_with_a_window();

        assert!(BackdropMask::flood_keeping(&photo, BackdropMask::BRIGHT, BackdropMask::POCKET_SHARE).vessel(100, 100));
        assert!(!BackdropMask::flood_keeping(&photo, BackdropMask::BRIGHT, 0.0001).vessel(100, 100));
        assert!(BackdropMask::flood_keeping(&photo, BackdropMask::BRIGHT, 0.0001).vessel(60, 60));
    }
}
