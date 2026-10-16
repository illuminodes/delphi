use std::error::Error;
use std::path::Path;

use image::RgbImage;

pub struct FlatSurvey {
    cell: u32,
}

impl FlatSurvey {
    const SHADES: [char; 6] = ['.', ':', '-', '=', '+', '#'];
    const STEP: f64 = 6.0;

    pub const fn new(cell: u32) -> Self {
        Self { cell }
    }

    pub fn print(&self, image: &Path) -> Result<(), Box<dyn Error>> {
        let image = image::open(image)?.to_rgb8();
        let (columns, rows) = (image.width() / self.cell, image.height() / self.cell);
        println!("cell {}px, {columns}x{rows}; rows show y0, then shade per column (. flat … # busy) and mean grey", self.cell);
        for row in 0..rows {
            let cells: Vec<(f64, f64)> = (0..columns).map(|column| self.measure(&image, column, row)).collect();
            let shades: String = cells.iter().map(|(energy, _)| Self::SHADES[((energy / Self::STEP) as usize).min(Self::SHADES.len() - 1)]).collect();
            let greys: Vec<String> = cells.iter().map(|(_, grey)| format!("{:3.0}", grey)).collect();
            println!("{:4} {shades}  {}", row * self.cell, greys.join(""));
        }
        Ok(())
    }

    fn measure(&self, image: &RgbImage, column: u32, row: u32) -> (f64, f64) {
        let grey = |x: u32, y: u32| {
            let [red, green, blue] = image.get_pixel(x, y).0;
            0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue)
        };
        let (left, top) = (column * self.cell, row * self.cell);
        let (mut energy, mut total, mut count) = (0.0, 0.0, 0.0);
        for y in top + 2..top + self.cell - 2 {
            for x in left + 2..left + self.cell - 2 {
                let centre = grey(x, y);
                energy += (grey(x + 2, y) - grey(x - 2, y)).abs().max((grey(x, y + 2) - grey(x, y - 2)).abs());
                total += centre;
                count += 1.0;
            }
        }
        (energy / count, total / count)
    }
}
