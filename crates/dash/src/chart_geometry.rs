use std::fmt::Write;

use crate::SeriesPoint;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartLayout {
    pub width: f64,
    pub height: f64,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl ChartLayout {
    const MIN_WIDTH: f64 = 160.0;
    const MIN_HEIGHT: f64 = 60.0;
    const Y_TICK_SPACING: f64 = 45.0;
    const X_TICK_SPACING: f64 = 120.0;

    /// A layout matching a measured box, with room for a price axis.
    #[must_use]
    pub fn for_series(width: f64, height: f64) -> Self {
        Self {
            width: width.max(Self::MIN_WIDTH),
            height: height.max(Self::MIN_HEIGHT),
            left: 62.0,
            right: 10.0,
            top: 8.0,
            bottom: 20.0,
        }
    }

    /// A layout matching a measured box, with no price axis.
    #[must_use]
    pub fn for_histogram(width: f64, height: f64) -> Self {
        Self {
            left: 8.0,
            right: 8.0,
            ..Self::for_series(width, height)
        }
    }

    #[must_use]
    pub fn y_tick_count(&self) -> usize {
        ((self.inner_height() / Self::Y_TICK_SPACING) as usize + 1).clamp(2, 6)
    }

    #[must_use]
    pub fn x_tick_count(&self) -> usize {
        ((self.inner_width() / Self::X_TICK_SPACING) as usize + 1).clamp(2, 7)
    }

    /// `count` grid line heights spread evenly from the top of the plot to its bottom, where a
    /// chart's price ticks fall, so an empty frame lines up with the chart that replaces it.
    #[must_use]
    pub fn grid_rows(&self, count: usize) -> Vec<f64> {
        let steps = count.max(2) - 1;
        (0..=steps)
            .map(|step| self.top + self.inner_height() * step as f64 / steps as f64)
            .collect()
    }

    /// `count` positions spread evenly across the plot, where a chart's time ticks fall.
    #[must_use]
    pub fn grid_columns(&self, count: usize) -> Vec<f64> {
        let steps = count.max(2) - 1;
        (0..=steps)
            .map(|step| self.left + self.inner_width() * step as f64 / steps as f64)
            .collect()
    }

    #[must_use]
    pub const fn standard() -> Self {
        Self {
            width: 640.0,
            height: 260.0,
            left: 68.0,
            right: 14.0,
            top: 14.0,
            bottom: 30.0,
        }
    }

    #[must_use]
    pub fn inner_width(&self) -> f64 {
        self.width - self.left - self.right
    }

    #[must_use]
    pub fn inner_height(&self) -> f64 {
        self.height - self.top - self.bottom
    }

    #[must_use]
    pub fn plot_bottom(&self) -> f64 {
        self.height - self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartTick {
    pub position: f64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartGeometry {
    layout: ChartLayout,
    points: Vec<SeriesPoint>,
    axis_low: f64,
    axis_high: f64,
    first_time: i64,
    time_span: f64,
}

impl ChartGeometry {
    const AXIS_PADDING: f64 = 0.08;
    const FLAT_PADDING: f64 = 0.005;

    #[must_use]
    pub fn new(points: &[SeriesPoint], layout: ChartLayout) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let first_time = points.first()?.stop_time();
        let last_time = points.last()?.stop_time();
        let low = points.iter().map(SeriesPoint::price).fold(f64::INFINITY, f64::min);
        let high = points.iter().map(SeriesPoint::price).fold(f64::NEG_INFINITY, f64::max);
        let padding = if high > low {
            (high - low) * Self::AXIS_PADDING
        } else {
            high * Self::FLAT_PADDING
        };
        Some(Self {
            layout,
            points: points.to_vec(),
            axis_low: low - padding,
            axis_high: high + padding,
            first_time,
            time_span: (last_time - first_time).max(1) as f64,
        })
    }

    #[must_use]
    pub const fn layout(&self) -> &ChartLayout {
        &self.layout
    }

    #[must_use]
    pub fn points(&self) -> &[SeriesPoint] {
        &self.points
    }

    #[must_use]
    pub fn x_of(&self, time: i64) -> f64 {
        self.layout.left + (time - self.first_time) as f64 / self.time_span * self.layout.inner_width()
    }

    #[must_use]
    pub fn y_of(&self, price: f64) -> f64 {
        let unit = (price - self.axis_low) / (self.axis_high - self.axis_low);
        self.layout.top + (1.0 - unit) * self.layout.inner_height()
    }

    #[must_use]
    pub fn position(&self, index: usize) -> Option<(f64, f64)> {
        let point = self.points.get(index)?;
        Some((self.x_of(point.stop_time()), self.y_of(point.price())))
    }

    #[must_use]
    pub fn line_path(&self) -> String {
        let mut path = String::new();
        for (index, point) in self.points.iter().enumerate() {
            let command = if index == 0 { 'M' } else { 'L' };
            let _ = write!(
                path,
                "{command}{:.1},{:.1}",
                self.x_of(point.stop_time()),
                self.y_of(point.price())
            );
        }
        path
    }

    #[must_use]
    pub fn area_path(&self) -> String {
        let mut path = self.line_path();
        let last = self.points.last().map_or(0, SeriesPoint::stop_time);
        let _ = write!(
            path,
            "L{:.1},{:.1}L{:.1},{:.1}Z",
            self.x_of(last),
            self.layout.plot_bottom(),
            self.x_of(self.first_time),
            self.layout.plot_bottom()
        );
        path
    }

    #[must_use]
    pub fn y_ticks(&self, count: usize) -> Vec<ChartTick> {
        let steps = count.max(2) - 1;
        (0..=steps)
            .map(|step| {
                let value = self.axis_low
                    + (self.axis_high - self.axis_low) * step as f64 / steps as f64;
                ChartTick {
                    position: self.y_of(value),
                    value,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn x_ticks(&self, count: usize) -> Vec<ChartTick> {
        let steps = count.max(2) - 1;
        (0..=steps)
            .map(|step| {
                let fraction = step as f64 / steps as f64;
                let time = self.first_time as f64 + self.time_span * fraction;
                ChartTick {
                    position: self.layout.left + fraction * self.layout.inner_width(),
                    value: time,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn index_at(&self, x: f64) -> usize {
        let fraction = ((x - self.layout.left) / self.layout.inner_width()).clamp(0.0, 1.0);
        let time = self.first_time as f64 + fraction * self.time_span;
        let after = self
            .points
            .partition_point(|point| (point.stop_time() as f64) < time);
        if after == 0 {
            return 0;
        }
        if after >= self.points.len() {
            return self.points.len() - 1;
        }
        let before_gap = time - self.points[after - 1].stop_time() as f64;
        let after_gap = self.points[after].stop_time() as f64 - time;
        if before_gap <= after_gap {
            after - 1
        } else {
            after
        }
    }

    #[must_use]
    pub fn low(&self) -> f64 {
        self.points.iter().map(SeriesPoint::price).fold(f64::INFINITY, f64::min)
    }

    #[must_use]
    pub fn high(&self) -> f64 {
        self.points.iter().map(SeriesPoint::price).fold(f64::NEG_INFINITY, f64::max)
    }

    #[must_use]
    pub fn change_percent(&self) -> f64 {
        let first = self.points.first().map_or(0.0, SeriesPoint::price);
        let last = self.points.last().map_or(0.0, SeriesPoint::price);
        if first > 0.0 {
            (last - first) / first * 100.0
        } else {
            0.0
        }
    }

    #[must_use]
    pub fn span_seconds(&self) -> i64 {
        self.time_span as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{ChartGeometry, ChartLayout};
    use crate::SeriesPoint;

    struct SeriesFixture;

    impl SeriesFixture {
        fn point(price: f64, stop_time: i64) -> SeriesPoint {
            serde_json::from_str(&format!(r#"{{"price":{price},"stop_time":{stop_time}}}"#))
                .expect("point")
        }

        fn series(values: &[(f64, i64)]) -> Vec<SeriesPoint> {
            values.iter().map(|(price, time)| Self::point(*price, *time)).collect()
        }

        fn geometry(values: &[(f64, i64)]) -> ChartGeometry {
            ChartGeometry::new(&Self::series(values), ChartLayout::standard()).expect("geometry")
        }
    }

    #[test]
    fn fewer_than_two_points_draw_no_chart() {
        let layout = ChartLayout::standard();

        assert!(ChartGeometry::new(&[], layout).is_none());
        assert!(ChartGeometry::new(&SeriesFixture::series(&[(1.0, 1)]), layout).is_none());
    }

    #[test]
    fn the_first_and_last_points_sit_on_the_plot_edges() {
        let chart = SeriesFixture::geometry(&[(10.0, 100), (20.0, 200), (15.0, 300)]);
        let layout = ChartLayout::standard();

        let first = chart.position(0).expect("first");
        let last = chart.position(2).expect("last");

        assert!((first.0 - layout.left).abs() < 1e-9);
        assert!((last.0 - (layout.width - layout.right)).abs() < 1e-9);
    }

    #[test]
    fn higher_prices_are_drawn_higher_and_stay_inside_the_plot() {
        let chart = SeriesFixture::geometry(&[(10.0, 100), (20.0, 200), (15.0, 300)]);
        let layout = ChartLayout::standard();

        let low = chart.position(0).expect("low").1;
        let high = chart.position(1).expect("high").1;

        assert!(high < low);
        assert!(high > layout.top && low < layout.plot_bottom());
    }

    #[test]
    fn a_flat_series_gets_a_finite_axis_and_a_centred_line() {
        let chart = SeriesFixture::geometry(&[(50.0, 1), (50.0, 2)]);
        let layout = ChartLayout::standard();

        let middle = layout.top + layout.inner_height() / 2.0;

        assert!((chart.position(0).expect("point").1 - middle).abs() < 1e-6);
        assert!(chart.line_path().find("NaN").is_none());
    }

    #[test]
    fn identical_timestamps_do_not_divide_by_zero() {
        let chart = SeriesFixture::geometry(&[(1.0, 5), (2.0, 5)]);

        assert!(!chart.line_path().contains("NaN"));
        assert!(!chart.line_path().contains("inf"));
    }

    #[test]
    fn the_line_path_starts_with_a_move_and_the_area_is_closed() {
        let chart = SeriesFixture::geometry(&[(10.0, 100), (20.0, 200)]);

        assert!(chart.line_path().starts_with('M'));
        assert_eq!(chart.line_path().matches('L').count(), 1);
        assert!(chart.area_path().ends_with('Z'));
    }

    #[test]
    fn ticks_run_from_the_axis_floor_to_its_ceiling() {
        let chart = SeriesFixture::geometry(&[(100.0, 0), (200.0, 100)]);

        let ticks = chart.y_ticks(5);

        assert_eq!(ticks.len(), 5);
        assert!(ticks[0].value < 100.0 && ticks[4].value > 200.0);
        assert!(ticks[0].position > ticks[4].position);
        let times = chart.x_ticks(3);
        assert_eq!(times.iter().map(|tick| tick.value as i64).collect::<Vec<_>>(), vec![0, 50, 100]);
    }

    #[test]
    fn the_pointer_selects_the_nearest_point_and_clamps_at_the_edges() {
        let chart = SeriesFixture::geometry(&[(1.0, 0), (2.0, 100), (3.0, 1_000)]);
        let layout = ChartLayout::standard();
        let at = |time: f64| layout.left + time / 1_000.0 * layout.inner_width();

        assert_eq!(chart.index_at(-50.0), 0);
        assert_eq!(chart.index_at(at(40.0)), 0);
        assert_eq!(chart.index_at(at(70.0)), 1);
        assert_eq!(chart.index_at(at(900.0)), 2);
        assert_eq!(chart.index_at(10_000.0), 2);
    }

    #[test]
    fn empty_grid_rows_fall_where_the_price_ticks_will() {
        let chart = SeriesFixture::geometry(&[(100.0, 0), (200.0, 100)]);
        let layout = ChartLayout::standard();

        let mut ticks: Vec<f64> = chart.y_ticks(4).iter().map(|tick| tick.position).collect();
        ticks.reverse();
        let rows = layout.grid_rows(4);

        assert_eq!(rows.len(), 4);
        assert!(rows.iter().zip(&ticks).all(|(row, tick)| (row - tick).abs() < 1e-9));

        let columns = layout.grid_columns(3);
        let times: Vec<f64> = chart.x_ticks(3).iter().map(|tick| tick.position).collect();
        assert!(columns.iter().zip(&times).all(|(column, time)| (column - time).abs() < 1e-9));
    }

    #[test]
    fn a_measured_layout_matches_its_box_but_never_collapses() {
        let layout = ChartLayout::for_series(900.0, 300.0);
        let tiny = ChartLayout::for_histogram(10.0, 5.0);

        assert!((layout.width - 900.0).abs() < 1e-9 && (layout.height - 300.0).abs() < 1e-9);
        assert!(tiny.inner_width() > 0.0 && tiny.inner_height() > 0.0);
        assert!(tiny.left < layout.left);
    }

    #[test]
    fn tick_counts_follow_the_available_space() {
        let small = ChartLayout::for_series(200.0, 80.0);
        let large = ChartLayout::for_series(1_400.0, 600.0);

        assert_eq!((small.x_tick_count(), small.y_tick_count()), (2, 2));
        assert_eq!((large.x_tick_count(), large.y_tick_count()), (7, 6));
    }

    #[test]
    fn the_range_summary_reports_extremes_and_change() {
        let chart = SeriesFixture::geometry(&[(100.0, 0), (80.0, 10), (110.0, 20)]);

        assert!((chart.low() - 80.0).abs() < 1e-9);
        assert!((chart.high() - 110.0).abs() < 1e-9);
        assert!((chart.change_percent() - 10.0).abs() < 1e-9);
        assert_eq!(chart.span_seconds(), 20);
    }
}
