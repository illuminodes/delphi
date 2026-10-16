use mantle_dash::CarveMask;

use crate::carve_class::CarveClass;

pub struct GrainReport {
    width: usize,
    height: usize,
}

impl GrainReport {
    const SOLID: f64 = 0.3;

    pub const fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    pub fn print(&self, mask: &CarveMask, stone: &[u8], carved: &[u8]) {
        for class in CarveClass::ALL {
            let pixels = self.pixels_of(mask, class);
            if pixels.is_empty() {
                continue;
            }
            let (under, cut): (Vec<f64>, Vec<f64>) = pixels.iter().map(|index| (Self::light(stone, *index), Self::light(carved, *index))).unzip();
            let clipped = pixels.iter().filter(|index| carved[*index * 4..*index * 4 + 3].contains(&255)).count();
            let hue = |pixels_from: &[u8]| Self::mean_colour(pixels_from, &pixels);
            println!(
                "{:<7} n={:<6} light stone {:>5.1} cut {:>5.1}  spread stone {:>4.1} cut {:>4.1}  r={:.2}  clipped {:>4.1}%  colour stone {} cut {}",
                class.name(),
                pixels.len(),
                Self::mean(&under),
                Self::mean(&cut),
                Self::deviation(&under),
                Self::deviation(&cut),
                Self::correlation(&under, &cut),
                100.0 * clipped as f64 / pixels.len() as f64,
                hue(stone),
                hue(carved)
            );
        }
    }

    fn pixels_of(&self, mask: &CarveMask, class: CarveClass) -> Vec<usize> {
        (0..self.height)
            .flat_map(|row| (0..self.width).map(move |column| (column, row)))
            .filter(|(column, row)| mask.cut_at(*column, *row) >= Self::SOLID && CarveClass::of(mask, *column, *row) == class)
            .map(|(column, row)| row * self.width + column)
            .collect()
    }

    fn light(pixels: &[u8], index: usize) -> f64 {
        let pixel = &pixels[index * 4..index * 4 + 3];
        0.2126 * f64::from(pixel[0]) + 0.7152 * f64::from(pixel[1]) + 0.0722 * f64::from(pixel[2])
    }

    fn mean_colour(pixels: &[u8], indices: &[usize]) -> String {
        let count = indices.len().max(1) as f64;
        let [red, green, blue] = [0, 1, 2].map(|channel| indices.iter().map(|index| f64::from(pixels[index * 4 + channel])).sum::<f64>() / count);
        format!("#{:02x}{:02x}{:02x}", red.round() as u8, green.round() as u8, blue.round() as u8)
    }

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len().max(1) as f64
    }

    fn deviation(values: &[f64]) -> f64 {
        let mean = Self::mean(values);
        (values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len().max(1) as f64).sqrt()
    }

    fn correlation(first: &[f64], second: &[f64]) -> f64 {
        let (first_mean, second_mean) = (Self::mean(first), Self::mean(second));
        let shared: f64 = first.iter().zip(second).map(|(a, b)| (a - first_mean) * (b - second_mean)).sum();
        let scale = (first.iter().map(|a| (a - first_mean).powi(2)).sum::<f64>() * second.iter().map(|b| (b - second_mean).powi(2)).sum::<f64>()).sqrt();
        shared / scale.max(1e-9)
    }
}
