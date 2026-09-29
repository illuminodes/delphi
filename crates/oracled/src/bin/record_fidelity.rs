use std::process::ExitCode;

use mantle::difficulty::BlockHeight;
use mantle_oracle::{AdmittedSample, CachedWindows, OraclePipeline, RangeSelection};

struct FidelityApp {
    data_dir: String,
    window: usize,
}

struct Comparison {
    start: BlockHeight,
    whole_outputs: usize,
    split_outputs: usize,
    whole_price: Option<f64>,
    split_price: Option<f64>,
}

impl Comparison {
    fn price_gap(&self) -> Option<f64> {
        let (whole, split) = (self.whole_price?, self.split_price?);
        Some((split - whole).abs() / whole)
    }

    fn cell(price: Option<f64>) -> String {
        price.map_or_else(|| "-".to_string(), |value| format!("{value:.0}"))
    }

    fn line(&self) -> String {
        format!(
            "{:>8} {:>9} {:>9} {:>10} {:>10} {:>8}",
            self.start,
            self.whole_outputs,
            self.split_outputs,
            Self::cell(self.whole_price),
            Self::cell(self.split_price),
            self.price_gap()
                .map_or_else(|| "-".to_string(), |gap| format!("{:.3}%", gap * 100.0)),
        )
    }
}

impl FidelityApp {
    fn parse() -> Self {
        let mut arguments = std::env::args().skip(1);
        let data_dir = arguments
            .next()
            .unwrap_or_else(|| "data/oracle/mainnet".to_string());
        let window = arguments
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(144);
        Self { data_dir, window }
    }

    fn execute() -> ExitCode {
        match Self::parse().run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(&self) -> Result<(), String> {
        let mut windows = CachedWindows::open(&self.data_dir).map_err(|error| error.to_string())?;
        let runs = windows.contiguous_runs(self.window);
        eprintln!("{} contiguous runs of at least {} blocks", runs.len(), self.window);
        println!(
            "{:>8} {:>9} {:>9} {:>10} {:>10} {:>8}",
            "start", "whole", "split", "whole $", "split $", "gap"
        );
        let mut gaps = Vec::new();
        for run in &runs {
            for tile in run.chunks_exact(self.window) {
                let (Some((start, _)), Some((stop, _))) = (tile.first(), tile.last()) else {
                    continue;
                };
                let comparison = Self::compare(&mut windows, *start, *stop)?;
                println!("{}", comparison.line());
                gaps.extend(comparison.price_gap());
            }
        }
        Self::summarize(&mut gaps);
        Ok(())
    }

    fn compare(
        windows: &mut CachedWindows,
        start: BlockHeight,
        stop: BlockHeight,
    ) -> Result<Comparison, String> {
        let mut whole = OraclePipeline::new();
        windows
            .feed(RangeSelection::Heights { start, stop }, &mut whole)
            .map_err(|error| error.to_string())?;
        let whole = whole.into_sample();

        let mut split = AdmittedSample::new();
        for height in start.to_u32()..=stop.to_u32() {
            let height = BlockHeight::new(height);
            let mut block = OraclePipeline::new();
            windows
                .feed(RangeSelection::Heights { start: height, stop: height }, &mut block)
                .map_err(|error| error.to_string())?;
            for output in block.sample().outputs() {
                split.push(output.amount, output.height, output.time);
            }
        }
        Ok(Comparison {
            start,
            whole_outputs: whole.len(),
            split_outputs: split.len(),
            whole_price: whole.price().ok().map(|report| report.central_price),
            split_price: split.price().ok().map(|report| report.central_price),
        })
    }

    #[allow(clippy::cast_precision_loss)]
    fn summarize(gaps: &mut [f64]) {
        if gaps.is_empty() {
            println!("\nno comparable windows");
            return;
        }
        gaps.sort_by(f64::total_cmp);
        let worst = gaps[gaps.len() - 1];
        let median = gaps[gaps.len() / 2];
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        println!(
            "\n{} windows: median gap {:.3}%, mean {:.3}%, worst {:.3}%",
            gaps.len(),
            median * 100.0,
            mean * 100.0,
            worst * 100.0
        );
    }
}

fn main() -> ExitCode {
    FidelityApp::execute()
}
