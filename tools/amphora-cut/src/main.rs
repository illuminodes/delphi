mod backdrop_mask;
mod glass_cast;

use std::error::Error;
use std::path::PathBuf;

use backdrop_mask::BackdropMask;
use glass_cast::GlassCast;
use image::{Rgba, RgbaImage};

enum Look {
    Photo,
    Glass(u32),
}

struct AmphoraCut {
    photo: PathBuf,
    output: PathBuf,
    look: Look,
    bright: f64,
    silhouette: Option<PathBuf>,
    pocket: f64,
}

impl AmphoraCut {
    const USAGE: &'static str = "usage: amphora-cut <photo.jpg> <out.png> [--glass <width>] [--bright <backdrop floor 0-255>] [--silhouette <mask.png>] [--pocket <smallest enclosed gap as a share of the photo>]";
    const KNOTS: usize = 48;

    fn from_args() -> Result<Self, Box<dyn Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let [photo, output, flags @ ..] = args.as_slice() else {
            return Err(Self::USAGE.into());
        };
        let mut cut = Self { photo: photo.into(), output: output.into(), look: Look::Photo, bright: BackdropMask::BRIGHT, silhouette: None, pocket: BackdropMask::POCKET_SHARE };
        for pair in flags.chunks(2) {
            match pair {
                [flag, width] if flag == "--glass" => cut.look = Look::Glass(width.parse()?),
                [flag, floor] if flag == "--bright" => cut.bright = floor.parse()?,
                [flag, path] if flag == "--silhouette" => cut.silhouette = Some(path.into()),
                [flag, share] if flag == "--pocket" => cut.pocket = share.parse()?,
                _ => return Err(Self::USAGE.into()),
            }
        }
        Ok(cut)
    }

    fn run(&self) -> Result<(), Box<dyn Error>> {
        let photo = image::open(&self.photo)?.to_rgb8();
        let mask = BackdropMask::flood_keeping(&photo, self.bright, self.pocket);
        let (left, top, right, bottom) = mask.bounds().ok_or("no vessel found")?;
        let frame = (left, top, right - left + 1, bottom - top + 1);
        let picture = match self.look {
            Look::Photo => Self::cut(&photo, &mask, frame),
            Look::Glass(width) => GlassCast::new(&mask, frame).render(width),
        };
        if let (Some(path), Look::Glass(width)) = (&self.silhouette, &self.look) {
            GlassCast::new(&mask, frame).silhouette(*width).save(path)?;
            println!("wrote {}", path.display());
        }
        picture.save(&self.output)?;
        println!("wrote {} ({}x{}, aspect {:.3})", self.output.display(), picture.width(), picture.height(), f64::from(picture.width()) / f64::from(picture.height()));
        Self::print_profile(&mask, frame);
        Ok(())
    }

    fn cut(photo: &image::RgbImage, mask: &BackdropMask, (left, top, width, height): (usize, usize, usize, usize)) -> RgbaImage {
        let mut cut = RgbaImage::new(width as u32, height as u32);
        for y in 0..height {
            for x in 0..width {
                let [red, green, blue] = photo.get_pixel((left + x) as u32, (top + y) as u32).0;
                let alpha = (mask.coverage(left + x, top + y) * 255.0).round() as u8;
                cut.put_pixel(x as u32, y as u32, Rgba([red, green, blue, alpha]));
            }
        }
        cut
    }

    fn print_profile(mask: &BackdropMask, (left, top, width, height): (usize, usize, usize, usize)) {
        let centre = left + width / 2;
        let half = width as f64 / 2.0;
        println!("profile (rise, reach) measured from the centre column outward:");
        for knot in 0..=Self::KNOTS {
            let rise = knot as f64 / Self::KNOTS as f64;
            let row = top + ((1.0 - rise) * (height - 1) as f64).round() as usize;
            let run = |step: i64| {
                let mut x = centre as i64;
                while x >= 0 && mask.vessel(x as usize, row) {
                    x += step;
                }
                (x - centre as i64).unsigned_abs() as f64
            };
            let reach = run(1).min(run(-1)) / half;
            println!("    ({rise:.3}, {reach:.3}),");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    AmphoraCut::from_args()?.run()
}
