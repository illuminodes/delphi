use std::fmt::Write;

use crate::{ChartLayout, FitNote};

/// One histogram bar in plot coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitBar {
    pub x: f64,
    pub width: f64,
    pub y: f64,
    pub height: f64,
    pub count: u32,
}

/// Plot coordinates for a fit: a log price axis over the implied-price range,
/// bars for the implied-price counts, and the rough stage's score curve.
#[derive(Debug, Clone, PartialEq)]
pub struct FitGeometry {
    layout: ChartLayout,
    low: f64,
    high: f64,
    counts: Vec<u32>,
    curve: Vec<(f64, f64)>,
}

impl FitGeometry {
    #[must_use]
    pub fn new(fit: &FitNote, layout: ChartLayout) -> Option<Self> {
        let (low, high) = fit.implied_range();
        if fit.implied_counts().is_empty() || !(low > 0.0 && high > low) {
            return None;
        }
        let curve: Vec<(f64, f64)> = fit
            .curve()
            .iter()
            .copied()
            .filter(|(price, _)| *price >= low && *price <= high)
            .collect();
        Some(Self {
            layout,
            low,
            high,
            counts: fit.implied_counts().to_vec(),
            curve,
        })
    }

    #[must_use]
    pub const fn layout(&self) -> &ChartLayout {
        &self.layout
    }

    #[must_use]
    pub const fn range(&self) -> (f64, f64) {
        (self.low, self.high)
    }

    #[must_use]
    pub fn contains(&self, price: f64) -> bool {
        price >= self.low && price <= self.high
    }

    #[must_use]
    pub fn x_of(&self, price: f64) -> f64 {
        let unit = (price / self.low).ln() / (self.high / self.low).ln();
        self.layout.left + unit.clamp(0.0, 1.0) * self.layout.inner_width()
    }

    #[must_use]
    pub fn price_at(&self, x: f64) -> f64 {
        let unit = ((x - self.layout.left) / self.layout.inner_width()).clamp(0.0, 1.0);
        self.low * (self.high / self.low).powf(unit)
    }

    fn y_of_share(&self, share: f64) -> f64 {
        self.layout.top + (1.0 - share.clamp(0.0, 1.0)) * self.layout.inner_height()
    }

    #[must_use]
    pub fn bars(&self) -> Vec<FitBar> {
        let tallest = self.counts.iter().copied().max().unwrap_or(0).max(1);
        let width = self.layout.inner_width() / self.counts.len() as f64;
        self.counts
            .iter()
            .enumerate()
            .map(|(index, count)| {
                let share = f64::from(*count) / f64::from(tallest);
                let y = self.y_of_share(share);
                FitBar {
                    x: self.layout.left + index as f64 * width,
                    width,
                    y,
                    height: self.layout.plot_bottom() - y,
                    count: *count,
                }
            })
            .collect()
    }

    /// The score curve, with scores below 0 drawn on the baseline.
    #[must_use]
    pub fn curve_path(&self) -> String {
        let mut path = String::new();
        for (index, (price, score)) in self.curve.iter().enumerate() {
            let command = if index == 0 { 'M' } else { 'L' };
            let _ = write!(path, "{command}{:.1},{:.1}", self.x_of(*price), self.y_of_share(*score));
        }
        path
    }

    /// Price ticks at evenly spaced positions on the log axis.
    #[must_use]
    pub fn ticks(&self, count: usize) -> Vec<(f64, f64)> {
        let steps = count.max(2) - 1;
        (0..=steps)
            .map(|step| {
                let x = self.layout.left + self.layout.inner_width() * step as f64 / steps as f64;
                (x, self.price_at(x))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::FitGeometry;
    use crate::{ChartLayout, FitNote, NoteKind};

    struct GeometryFixture;

    impl GeometryFixture {
        fn fit(counts: &str) -> FitNote {
            let content = format!(
                r#"{{"source":"chain","price":100.0,"curve":{{"first_price":200.0,"step":0.9,"scores":[0.0,0.2,0.5,0.9,1.0,0.7,0.3,0.1,-0.4,0.0]}},"implied":{{"low":80.0,"high":125.0,"counts":{counts}}}}}"#
            );
            let note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(content)
                .tag("w", "fit-chain-1h")
                .build();
            FitNote::from_note(&note).expect("fit")
        }

        fn geometry(counts: &str) -> FitGeometry {
            FitGeometry::new(&Self::fit(counts), ChartLayout::standard()).expect("geometry")
        }
    }

    #[test]
    fn the_range_edges_map_to_the_plot_edges() {
        let chart = GeometryFixture::geometry("[1,2,3]");
        let layout = ChartLayout::standard();

        assert!((chart.x_of(80.0) - layout.left).abs() < 1e-9);
        assert!((chart.x_of(125.0) - (layout.width - layout.right)).abs() < 1e-9);
        assert!((chart.price_at(chart.x_of(100.0)) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn the_tallest_bar_reaches_the_top_and_bars_tile_the_width() {
        let chart = GeometryFixture::geometry("[1,4,2,0]");
        let layout = ChartLayout::standard();
        let bars = chart.bars();

        assert_eq!(bars.len(), 4);
        assert!((bars[1].y - layout.top).abs() < 1e-9);
        assert!(bars[3].height.abs() < 1e-9);
        let right = bars[3].x + bars[3].width;
        assert!((right - (layout.width - layout.right)).abs() < 1e-9);
    }

    #[test]
    fn the_curve_keeps_only_prices_inside_the_range() {
        let chart = GeometryFixture::geometry("[1]");

        assert!(chart.curve_path().starts_with('M'));
        assert!(!chart.curve_path().contains("NaN"));
        assert!(chart.contains(100.0));
        assert!(!chart.contains(200.0));
    }

    #[test]
    fn an_empty_histogram_draws_nothing() {
        assert!(FitGeometry::new(&GeometryFixture::fit("[]"), ChartLayout::standard()).is_none());
    }

    #[test]
    fn ticks_rise_across_the_axis() {
        let ticks = GeometryFixture::geometry("[1]").ticks(5);

        assert_eq!(ticks.len(), 5);
        assert!(ticks.windows(2).all(|pair| pair[1].1 > pair[0].1));
        assert!((ticks[0].1 - 80.0).abs() < 1e-9);
    }
}
