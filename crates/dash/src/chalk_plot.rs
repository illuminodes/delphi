use crate::{BandFlow, ChalkStroke, ChartGeometry, ChartLayout, ChartTick, SeriesPoint, StoneRecipe, VeinBranches, VeinPath};

#[derive(Debug, Clone, PartialEq)]
pub struct ChalkRule {
    pub tick: ChartTick,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChalkPlot {
    chart: ChartGeometry,
    seed: u64,
}

impl ChalkPlot {
    const RULE_WOBBLE: f64 = 0.7;
    const RING_RADIUS: f64 = 10.0;
    const RING_WOBBLE: f64 = 1.4;
    const SALT: u64 = 0x2545_F491_4F6C_DD1D;

    #[must_use]
    pub fn layout(width: f64, height: f64) -> ChartLayout {
        ChartLayout {
            left: 66.0,
            right: 20.0,
            top: 18.0,
            bottom: 28.0,
            ..ChartLayout::for_series(width, height)
        }
    }

    #[must_use]
    pub fn new(points: &[SeriesPoint], layout: ChartLayout, seed: u64) -> Option<Self> {
        Some(Self {
            chart: ChartGeometry::new(points, layout)?,
            seed,
        })
    }

    #[must_use]
    pub const fn chart(&self) -> &ChartGeometry {
        &self.chart
    }

    #[must_use]
    pub fn last_index(&self) -> usize {
        self.chart.points().len() - 1
    }

    #[must_use]
    pub fn vein_path(&self) -> Option<VeinPath> {
        VeinPath::new(&self.positions())
    }

    #[must_use]
    pub fn band_flow(&self) -> Option<BandFlow> {
        BandFlow::new(&self.positions(), self.chart.layout().width)
    }

    #[must_use]
    pub fn vein_branches(&self, recipe: StoneRecipe) -> VeinBranches {
        VeinBranches::grow(&self.positions(), recipe, self.seed)
    }

    fn positions(&self) -> Vec<(f64, f64)> {
        (0..self.chart.points().len())
            .filter_map(|index| self.chart.position(index))
            .collect()
    }

    #[must_use]
    pub fn rules(&self) -> Vec<ChalkRule> {
        let layout = self.chart.layout();
        let (left, right) = (layout.left, layout.width - layout.right);
        self.chart
            .y_ticks(layout.y_tick_count())
            .into_iter()
            .enumerate()
            .map(|(index, tick)| ChalkRule {
                tick,
                path: self.stroke(index as u64 + 1, Self::RULE_WOBBLE).trace(&[
                    (left, tick.position),
                    (right, tick.position),
                ]),
            })
            .collect()
    }

    #[must_use]
    pub fn ring(&self, index: usize) -> Option<String> {
        let centre = self.chart.position(index)?;
        Some(
            self.stroke(1_000 + index as u64, Self::RING_WOBBLE)
                .loop_around(centre, Self::RING_RADIUS),
        )
    }

    #[must_use]
    pub fn cursor(&self, index: usize) -> Option<String> {
        let (x, _) = self.chart.position(index)?;
        let layout = self.chart.layout();
        Some(
            self.stroke(2_000 + index as u64, Self::RULE_WOBBLE)
                .trace(&[(x, layout.top), (x, layout.plot_bottom())]),
        )
    }

    fn stroke(&self, salt: u64, wobble: f64) -> ChalkStroke {
        ChalkStroke::new(self.seed ^ salt.wrapping_mul(Self::SALT), wobble)
    }
}

#[cfg(test)]
mod tests {
    use super::ChalkPlot;
    use crate::SampleSeries;

    struct PlotFixture;

    impl PlotFixture {
        fn plot(count: usize) -> Option<ChalkPlot> {
            ChalkPlot::new(&SampleSeries::new(4, count).points(), ChalkPlot::layout(640.0, 280.0), 9)
        }
    }

    #[test]
    fn fewer_than_two_points_draw_no_plot() {
        assert!(PlotFixture::plot(1).is_none());
        assert!(PlotFixture::plot(2).is_some());
    }

    #[test]
    fn the_vein_runs_through_the_last_price() {
        let plot = PlotFixture::plot(144).unwrap();
        let (x, y) = plot.chart().position(plot.last_index()).unwrap();
        let point = plot.vein_path().unwrap().nearest(x, y).unwrap();

        assert!(point.offset.abs() < 1e-9 && (point.strength - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_bands_are_centred_on_the_last_price() {
        let plot = PlotFixture::plot(144).unwrap();
        let (x, y) = plot.chart().position(plot.last_index()).unwrap();

        assert!(plot.band_flow().unwrap().offset(x, y).abs() < 4.0);
    }

    #[test]
    fn rules_follow_the_price_ticks_and_each_wobbles_differently() {
        let plot = PlotFixture::plot(144).unwrap();
        let rules = plot.rules();

        assert_eq!(rules.len(), plot.chart().layout().y_tick_count());
        assert!(rules.windows(2).all(|pair| pair[0].path != pair[1].path));
    }

    #[test]
    fn the_same_plot_is_drawn_the_same_way_twice() {
        let first = PlotFixture::plot(60).unwrap();
        let second = PlotFixture::plot(60).unwrap();

        assert_eq!(first.rules(), second.rules());
        assert_eq!(first.ring(10), second.ring(10));
    }

    #[test]
    fn rings_and_cursors_exist_only_for_real_points() {
        let plot = PlotFixture::plot(30).unwrap();

        assert!(plot.ring(plot.last_index()).is_some());
        assert!(plot.cursor(0).is_some());
        assert!(plot.ring(30).is_none());
        assert!(plot.cursor(30).is_none());
    }

    #[test]
    fn the_layout_leaves_room_for_the_ring_inside_the_slab() {
        let layout = ChalkPlot::layout(640.0, 280.0);

        assert!(layout.top >= 12.0 && layout.right >= 12.0);
    }
}
