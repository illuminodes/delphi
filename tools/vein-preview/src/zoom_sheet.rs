use std::error::Error;
use std::path::Path;

use image::{Rgba, RgbaImage};
use delphi_dash::{CarveMask, CutLip, VeeCut};

use crate::etched_preview::EtchedPreview;

type Panel<'a> = Box<dyn Fn(usize, usize) -> [u8; 3] + 'a>;

pub struct ZoomSheet {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

impl ZoomSheet {
    const ZOOM: u32 = 10;
    const GAP: u32 = 4;

    pub const fn new(left: usize, top: usize, width: usize, height: usize) -> Self {
        Self { left, top, width, height }
    }

    pub fn save(&self, preview: &EtchedPreview, output: &Path) -> Result<(), Box<dyn Error>> {
        let mask = preview.mask()?;
        let stone = preview.stone()?;
        let etched = preview.etched(&stone, &mask);
        let recipe = preview.kind().etch();
        let vee = VeeCut::new(&mask, recipe.vee_slope, recipe.vee_floor, recipe.ambient);
        let panels: [Panel; 5] = [
            Box::new(|column, row| Self::colour(&stone, mask.width(), column, row)),
            Box::new(|column, row| Self::mask_colour(&mask, column, row)),
            Box::new(|column, row| Self::grey(vee.light(column, row) * 128.0)),
            Box::new(|column, row| Self::grey(vee.sink(column, row) * 255.0)),
            Box::new(|column, row| Self::colour(&etched, mask.width(), column, row)),
        ];
        let panel_width = self.width as u32 * Self::ZOOM;
        let panel_height = self.height as u32 * Self::ZOOM;
        let mut sheet = RgbaImage::from_pixel(panel_width, (panel_height + Self::GAP) * panels.len() as u32, Rgba([255, 0, 255, 255]));
        for (index, panel) in panels.iter().enumerate() {
            let offset = index as u32 * (panel_height + Self::GAP);
            for (x, y, pixel) in sheet.enumerate_pixels_mut().filter(|(_, y, _)| (offset..offset + panel_height).contains(y)) {
                let [red, green, blue] = panel(self.left + (x / Self::ZOOM) as usize, self.top + ((y - offset) / Self::ZOOM) as usize);
                *pixel = Rgba([red, green, blue, 255]);
            }
        }
        sheet.save(output)?;
        println!("rows: stone, mask (red cut, green gilt, blue incised/groove), vee light (mid grey = flat), cut depth, result");
        self.print_row(&mask, &vee, &stone, &etched);
        Self::print_stray(&mask, &stone, &etched);
        Ok(())
    }

    const TRACE: f64 = 1e-3;

    fn print_stray(mask: &CarveMask, stone: &[u8], etched: &[u8]) {
        let lip = CutLip::new(mask);
        let stray: Vec<(usize, usize)> = (0..mask.height())
            .flat_map(|row| (0..mask.width()).map(move |column| (column, row)))
            .filter(|(column, row)| mask.cut_at(*column, *row) < Self::TRACE && lip.at(mask, *column, *row) < Self::TRACE)
            .filter(|(column, row)| Self::colour(stone, mask.width(), *column, *row) != Self::colour(etched, mask.width(), *column, *row))
            .collect();
        println!("untouched-by-mask pixels that changed: {} (first {:?})", stray.len(), stray.iter().take(5).collect::<Vec<_>>());
    }

    fn print_row(&self, mask: &CarveMask, vee: &VeeCut, stone: &[u8], etched: &[u8]) {
        let row = self.top + self.height / 2;
        let lip = CutLip::new(mask);
        println!("row {row}: column cut lip light sink stone result");
        for column in self.left..self.left + self.width {
            let [red, green, blue] = Self::colour(etched, mask.width(), column, row);
            let [stone_red, stone_green, stone_blue] = Self::colour(stone, mask.width(), column, row);
            println!(
                "{column:>4} {:.2e} {:.2} {:.2} {:.2} #{stone_red:02x}{stone_green:02x}{stone_blue:02x} #{red:02x}{green:02x}{blue:02x}",
                mask.cut_at(column, row),
                lip.at(mask, column, row),
                vee.light(column, row),
                vee.sink(column, row)
            );
        }
    }

    fn colour(pixels: &[u8], width: usize, column: usize, row: usize) -> [u8; 3] {
        let index = (row * width + column) * 4;
        [pixels[index], pixels[index + 1], pixels[index + 2]]
    }

    fn mask_colour(mask: &CarveMask, column: usize, row: usize) -> [u8; 3] {
        let level = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        [level(mask.cut_at(column, row)), level(mask.gilt_at(column, row)), level(mask.incised_at(column, row).max(mask.grooved_at(column, row)))]
    }

    fn grey(value: f64) -> [u8; 3] {
        [value.round().clamp(0.0, 255.0) as u8; 3]
    }
}
