mod carve_class;
mod etched_preview;
mod flat_survey;
mod fume_film;
mod glyph_letterer;
mod grain_report;
mod oracle_film;
mod row_probe;
mod zoom_sheet;

use std::error::Error;
use std::path::PathBuf;

use etched_preview::EtchedPreview;
use flat_survey::FlatSurvey;
use fume_film::FumeFilm;
use oracle_film::OracleFilm;
use row_probe::RowProbe;
use zoom_sheet::ZoomSheet;

use image::RgbaImage;
use mantle_dash::{ChalkPlot, MarbleKind, MarbleStone, SampleSeries};

struct PreviewRequest {
    etched: bool,
    kind: MarbleKind,
    output: PathBuf,
    seed: u64,
    width: usize,
    height: usize,
}

impl PreviewRequest {
    const USAGE: &'static str = "usage: vein-preview [--etched] <nero|parian|aurum> <out.png> [seed] [width] [height]";

    fn from_args() -> Result<Self, Box<dyn Error>> {
        let mut args: Vec<String> = std::env::args().skip(1).collect();
        let etched = args.first().is_some_and(|first| first == "--etched");
        if etched {
            args.remove(0);
        }
        if args.len() < 2 {
            return Err(Self::USAGE.into());
        }
        let kind = MarbleKind::ALL
            .into_iter()
            .find(|kind| kind.key() == args[0])
            .ok_or_else(|| format!("unknown stone {}", args[0]))?;
        let number = |at: usize, default: usize| args.get(at).map_or(Ok(default), |text| text.parse::<usize>());
        Ok(Self {
            etched,
            kind,
            output: PathBuf::from(&args[1]),
            seed: number(2, 7)? as u64,
            width: number(3, 720)?,
            height: number(4, 300)?,
        })
    }

    fn run(&self) -> Result<(), Box<dyn Error>> {
        if self.etched {
            EtchedPreview::new(self.kind, self.seed, self.width, self.height).save(&self.output)?;
            println!("wrote {}", self.output.display());
            return Ok(());
        }
        let layout = ChalkPlot::layout(self.width as f64, self.height as f64);
        let plot = ChalkPlot::new(&SampleSeries::new(self.seed, 144).points(), layout, self.seed).ok_or("no chart")?;
        let stone = MarbleStone::new(self.kind.recipe(), self.seed);
        let pixels = stone.render(
            &plot.vein_path().ok_or("no path")?,
            &plot.band_flow().ok_or("no flow")?,
            &plot.vein_branches(stone.recipe()),
            self.width,
            self.height,
            1.0,
        );
        RgbaImage::from_raw(self.width as u32, self.height as u32, pixels)
            .ok_or("bad render")?
            .save(&self.output)?;
        println!("wrote {}", self.output.display());
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|flag| flag == "--fumes") {
        let [_, output, width, height, times @ ..] = args.as_slice() else {
            return Err("usage: vein-preview --fumes <out.png> <width> <height> [h<harvest seconds>] <seconds>...".into());
        };
        return FumeFilm::new(width.parse()?, height.parse()?, times)?.save(std::path::Path::new(output));
    }
    if args.first().is_some_and(|flag| flag == "--oracle") {
        let [_, output, width, height, times @ ..] = args.as_slice() else {
            return Err("usage: vein-preview --oracle <out.png> <width> <height> <seconds>...".into());
        };
        return OracleFilm::new(width.parse()?, height.parse()?, times)?.save(std::path::Path::new(output));
    }
    if let [flag, image, cell] = args.as_slice() {
        if flag == "--flat" {
            return FlatSurvey::new(cell.parse()?).print(std::path::Path::new(image));
        }
    }
    if let [flag, stone, output, left, top, width, height] = args.as_slice().iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        if *flag != "--zoom" {
            return Err("usage: vein-preview --zoom <stone> <out.png> <left> <top> <width> <height>".into());
        }
        let kind = MarbleKind::ALL.into_iter().find(|kind| kind.key() == *stone).ok_or("unknown stone")?;
        let preview = EtchedPreview::new(kind, 7, 900, 560);
        return ZoomSheet::new(left.parse()?, top.parse()?, width.parse()?, height.parse()?).save(&preview, std::path::Path::new(output));
    }
    if let [flag, image, row, from, to] = args.as_slice() {
        if flag == "--row" {
            return RowProbe::row(row.parse()?, from.parse()?, to.parse()?).print(std::path::Path::new(image));
        }
        if flag == "--column" {
            return RowProbe::column(row.parse()?, from.parse()?, to.parse()?).print(std::path::Path::new(image));
        }
    }
    PreviewRequest::from_args()?.run()
}
