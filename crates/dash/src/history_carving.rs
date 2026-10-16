use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveHeading, CarveText, CarveWeight, Carving, ChartGeometry, ChartLayout, CivilTime, SampleSeries};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryCarving {
    unit: f64,
    seed: u64,
}

impl HistoryCarving {
    const TABS: [&'static str; 4] = ["LIVE", "HOURLY", "DAILY", "AUDIT"];
    const CHOSEN: usize = 2;
    const POINTS: usize = 168;

    #[must_use]
    pub const fn new(unit: f64, seed: u64) -> Self {
        Self { unit, seed }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("HISTORY", "7 days", unit).carve(area);
        let tab_y = body.top + 12.0 * unit;
        let mut tab_x = body.left;
        for (index, tab) in Self::TABS.iter().enumerate() {
            let mut text = CarveText::new(*tab, CarveFont::Display, 11.0 * unit, tab_x, tab_y).tracking(0.16).depth(0.7);
            if index == Self::CHOSEN {
                text = text.gilt().depth(1.0);
                let bounds = text.bounds();
                carvings.push(CarveGroove::line((bounds.left, tab_y + 6.0 * unit), (bounds.right, tab_y + 6.0 * unit), 0.0).weighted(CarveWeight::Rule, unit).gilt().into());
            }
            tab_x = text.bounds().right + 22.0 * unit;
            carvings.push(text.into());
        }
        let plot = CarveBox::new(body.left, tab_y + 18.0 * unit, body.right, body.bottom);
        carvings.extend(self.chart(plot));
        carvings
    }

    fn chart(&self, plot: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let layout = ChartLayout { width: plot.width(), height: plot.height(), left: 58.0 * unit, right: 16.0 * unit, top: 10.0 * unit, bottom: 22.0 * unit };
        let Some(chart) = ChartGeometry::new(&SampleSeries::new(self.seed, Self::POINTS).points(), layout) else {
            return Vec::new();
        };
        let at = |(x, y): (f64, f64)| (plot.left + x, plot.top + y);
        let mut carvings = Vec::new();
        for tick in chart.y_ticks(layout.y_tick_count()) {
            let y = plot.top + tick.position;
            for dash in CarveGroove::dashed((plot.left + layout.left, y), (plot.right - layout.right, y), 0.0, 3.0 * unit, 6.0 * unit) {
                carvings.push(dash.weighted(CarveWeight::Hair, unit).into());
            }
            let label = format!("{:.1}k", tick.value / 1000.0);
            carvings.push(CarveText::new(label, CarveFont::Figures, 10.5 * unit, plot.left + layout.left - 8.0 * unit, y + 3.5 * unit).align(CarveAlign::End).depth(0.7).into());
        }
        for tick in chart.x_ticks(layout.x_tick_count()) {
            let label = CivilTime::from_unix(tick.value as i64).date_label();
            carvings.push(CarveText::new(label, CarveFont::Figures, 10.5 * unit, plot.left + tick.position, plot.bottom - 4.0 * unit).align(CarveAlign::Centre).depth(0.7).into());
        }
        let line: Vec<(f64, f64)> = (0..chart.points().len()).filter_map(|index| chart.position(index)).map(at).collect();
        let last = line.last().copied();
        carvings.push(CarveGroove::new(line, 0.0).weighted(CarveWeight::Bold, unit).into());
        if let Some(centre) = last {
            carvings.push(CarveGroove::ring(centre, 7.0 * unit, 0.0).weighted(CarveWeight::Bold, unit).gilt().into());
        }
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryCarving;
    use crate::{CarveBox, Carving};

    #[test]
    fn the_history_section_keeps_inside_its_area() {
        let area = CarveBox::new(300.0, 390.0, 1020.0, 640.0);
        let carvings = HistoryCarving::new(1.0, 7).carve(area);

        assert!(carvings.iter().all(|carving| area.grow(2.0).contains(carving.bounds())), "{:?}", carvings.iter().map(Carving::bounds).find(|bounds| !area.grow(2.0).contains(*bounds)));
    }

    #[test]
    fn the_price_line_is_carved_as_one_groove() {
        let carvings = HistoryCarving::new(1.0, 7).carve(CarveBox::new(0.0, 0.0, 700.0, 260.0));

        assert!(carvings.iter().any(|carving| matches!(carving, Carving::Groove(groove) if groove.points.len() == 168)));
    }
}
