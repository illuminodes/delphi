use std::error::Error;
use std::path::Path;

pub struct RowProbe {
    across: bool,
    line: u32,
    from: u32,
    to: u32,
}

impl RowProbe {
    pub const fn row(line: u32, from: u32, to: u32) -> Self {
        Self { across: true, line, from, to }
    }

    pub const fn column(line: u32, from: u32, to: u32) -> Self {
        Self { across: false, line, from, to }
    }

    pub fn print(&self, image: &Path) -> Result<(), Box<dyn Error>> {
        let image = image::open(image)?.to_rgb8();
        let limit = if self.across { image.width() } else { image.height() };
        let tones: Vec<String> = (self.from..self.to.min(limit))
            .map(|step| {
                let (x, y) = if self.across { (step, self.line) } else { (self.line, step) };
                let [red, green, blue] = image.get_pixel(x, y).0;
                format!("{step}:#{red:02x}{green:02x}{blue:02x}")
            })
            .collect();
        println!("{}", tones.join(" "));
        Ok(())
    }
}
