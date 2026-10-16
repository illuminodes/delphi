use crate::{FumeBowl, FumeJar, FumeRoute};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FumeLayout {
    pub width: f64,
    pub height: f64,
}

impl FumeLayout {
    const DESIGN_WIDTH: f64 = 1200.0;
    const DESIGN_HEIGHT: f64 = 800.0;
    const CLEFT: (f64, f64) = (0.5, 0.88);
    const CLEFT_SPAN: f64 = 0.46;
    const HANG_Y: f64 = 0.1;
    const JAR_HEIGHT: f64 = 0.3;
    const JAR_SPACING: (f64, f64) = (0.15, 0.22);
    const BEAM_REACH: f64 = 1.7;
    const BEAM_GAP: f64 = 0.035;
    const RISE: f64 = 0.45;
    const BOW: f64 = 0.6;
    const INTO_MOUTH: f64 = 0.08;
    const STRAIGHT: f64 = 0.3;
    const BOWL_STANDS: f64 = 1.15;
    const BOWL_CLEARANCE: f64 = 0.2;
    const POUR_DROP: f64 = 0.8;

    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn unit(&self) -> f64 {
        (self.width / Self::DESIGN_WIDTH).min(self.height / Self::DESIGN_HEIGHT).max(0.25)
    }

    #[must_use]
    pub fn cleft(&self) -> (f64, f64) {
        (self.width * Self::CLEFT.0, self.height * Self::CLEFT.1)
    }

    #[must_use]
    pub fn cleft_span(&self) -> f64 {
        (self.width * Self::CLEFT_SPAN).min(self.height * 0.75)
    }

    #[must_use]
    pub fn hang(&self) -> f64 {
        self.height * Self::HANG_Y
    }

    #[must_use]
    pub fn beam(&self) -> (f64, f64, f64) {
        let centre = self.cleft().0;
        let reach = self.jar_spacing() * Self::BEAM_REACH;
        (centre - reach, centre + reach, self.hang() - self.height * Self::BEAM_GAP)
    }

    #[must_use]
    pub fn jar_size(&self) -> (f64, f64) {
        let height = (self.height * Self::JAR_HEIGHT).min(self.jar_spacing() * 0.75 / FumeJar::ASPECT);
        (height, height * FumeJar::ASPECT)
    }

    #[must_use]
    pub fn jar_centre(&self, slot: usize) -> f64 {
        self.cleft().0 + (slot.min(2) as f64 - 1.0) * self.jar_spacing()
    }

    fn jar_spacing(&self) -> f64 {
        (self.width * Self::JAR_SPACING.0).min(self.height * Self::JAR_SPACING.1)
    }

    #[must_use]
    pub fn source_anywhere(&self, across: f64) -> (f64, f64) {
        (self.cleft().0 + (across.clamp(0.0, 1.0) - 0.5) * self.cleft_span() * 0.8, self.cleft().1)
    }

    #[must_use]
    pub fn bowl(&self) -> FumeBowl {
        let height = self.jar_size().0 * Self::BOWL_STANDS;
        let base = self.cleft().1 - 12.0 * self.unit();
        let reach = FumeBowl::reach(height);
        let centre = (self.cleft().0 + self.cleft_span() * 0.5 + reach * (1.0 + Self::BOWL_CLEARANCE)).min(self.width - reach * 1.05);
        FumeBowl::standing(centre, base, height)
    }

    #[must_use]
    pub fn pour(&self, jar: &FumeJar, bowl: &FumeBowl) -> FumeRoute {
        let (mouth_x, mouth_y) = jar.mouth();
        let landing = (bowl.centre, bowl.rim + bowl.depth * 0.35);
        let drop = (landing.1 - mouth_y) * Self::POUR_DROP;
        FumeRoute::bezier([(mouth_x, mouth_y - jar.height * 0.04), (mouth_x, mouth_y + drop), (landing.0, landing.1 - drop), landing])
    }

    #[must_use]
    pub fn route(&self, origin: (f64, f64), jar: &FumeJar, lean: f64) -> FumeRoute {
        let (mouth_x, mouth_y) = jar.mouth();
        let under = (mouth_x, mouth_y + jar.height * Self::STRAIGHT);
        let climb = origin.1 - under.1;
        let bow = lean.clamp(-0.5, 0.5) * jar.width * Self::BOW;
        FumeRoute::bezier([origin, (origin.0 + bow, origin.1 - climb * Self::RISE), (mouth_x, under.1 + climb * Self::RISE), under])
            .then_to((mouth_x, mouth_y - jar.height * Self::INTO_MOUTH))
    }
}

#[cfg(test)]
mod tests {
    use super::FumeLayout;
    use crate::{FumeFate, FumeJar, FumeRoute};

    fn jar(layout: &FumeLayout, slot: usize) -> FumeJar {
        let (height, width) = layout.jar_size();
        FumeJar::new(FumeFate::JARS[slot], layout.jar_centre(slot), layout.hang(), height, width)
    }

    const SOURCES: [f64; 3] = [0.0, 0.5, 1.0];

    fn routes(layout: &FumeLayout, slot: usize, lean: f64) -> Vec<FumeRoute> {
        let jar = jar(layout, slot);
        SOURCES.iter().map(|across| layout.route(layout.source_anywhere(*across), &jar, lean)).collect()
    }

    fn route(layout: &FumeLayout, slot: usize, lean: f64) -> FumeRoute {
        routes(layout, slot, lean).swap_remove(0)
    }

    fn points(route: &FumeRoute) -> Vec<(f64, f64)> {
        (0..=400).map(|step| route.at(f64::from(step) / 400.0)).collect()
    }

    #[test]
    fn the_jars_hang_side_by_side_from_the_beam_right_over_the_cleft() {
        for (width, height) in [(1200.0, 800.0), (800.0, 900.0), (1600.0, 700.0)] {
            let layout = FumeLayout::new(width, height);
            let (from, to, beam) = layout.beam();
            let (cleft, half_span) = (layout.cleft().0, layout.cleft_span() * 0.5);
            for slot in 0..3 {
                let jar = jar(&layout, slot);
                assert!(beam > 0.0 && jar.top() > beam);
                assert!(jar.mouth().1 < layout.cleft().1 - height * 0.3);
                assert!(jar.left() > from && jar.left() + jar.width < to);
                assert!((jar.centre - cleft).abs() < half_span - jar.width * 0.25, "{width}x{height} slot {slot}");
            }
            for slot in 1..3 {
                assert!(jar(&layout, slot).left() > jar(&layout, slot - 1).left() + jar(&layout, slot - 1).width);
            }
        }
    }

    #[test]
    fn every_route_rises_into_its_jar_mouth() {
        let layout = FumeLayout::new(1200.0, 800.0);
        for slot in 0..3 {
            let jar = jar(&layout, slot);
            let end = route(&layout, slot, 0.3).at(1.0);

            assert!(jar.outline().contains(end.0, end.1), "slot {slot}: {end:?}");
        }
    }

    #[test]
    fn a_route_only_ever_rises_and_bends_gently_ending_straight_up() {
        let layout = FumeLayout::new(1200.0, 800.0);
        for slot in 0..3 {
            for (lean, route) in [-0.5, 0.0, 0.5].into_iter().flat_map(|lean| routes(&layout, slot, lean).into_iter().map(move |route| (lean, route))) {
                let points = points(&route);
                let tightest = points
                    .windows(3)
                    .map(|three| {
                        let one = (three[1].1 - three[0].1).atan2(three[1].0 - three[0].0);
                        let two = (three[2].1 - three[1].1).atan2(three[2].0 - three[1].0);
                        let turn = (two - one).abs();
                        let step = (three[2].0 - three[1].0).hypot(three[2].1 - three[1].1);
                        step / turn.min(std::f64::consts::TAU - turn).max(1e-9)
                    })
                    .fold(f64::INFINITY, f64::min);
                let last = (points[points.len() - 1].0 - points[points.len() - 8].0, points[points.len() - 1].1 - points[points.len() - 8].1);

                assert!(points.windows(2).all(|pair| pair[1].1 <= pair[0].1 + 1e-6), "slot {slot} lean {lean} dips");
                assert!(tightest > layout.height * 0.06, "slot {slot} lean {lean}: bends with radius {tightest}");
                assert!(last.0.abs() < -last.1 * 0.2, "slot {slot} lean {lean}: {last:?}");
            }
        }
    }

    #[test]
    fn the_last_stretch_climbs_straight_up_under_the_mouth() {
        let layout = FumeLayout::new(1200.0, 800.0);
        for slot in 0..3 {
            let jar = jar(&layout, slot);
            for route in [-0.5, 0.0, 0.5].into_iter().flat_map(|lean| routes(&layout, slot, lean)) {
                let points = points(&route);
                let below = jar.mouth().1 + jar.height * 0.6;
                let stretch: Vec<&(f64, f64)> = points.iter().filter(|point| point.1 < below).collect();
                let widest = stretch.iter().map(|point| (point.0 - jar.centre).abs()).fold(0.0, f64::max);

                assert!(stretch.len() > 20, "slot {slot}");
                assert!(widest < jar.width * 0.35, "slot {slot}: {widest}");
            }
        }
    }

    #[test]
    fn fumes_from_either_side_of_the_cleft_cross_on_the_way_to_the_outer_jars() {
        let layout = FumeLayout::new(1200.0, 800.0);
        let to_right = layout.route(layout.source_anywhere(0.0), &jar(&layout, 2), 0.0);
        let to_left = layout.route(layout.source_anywhere(1.0), &jar(&layout, 0), 0.0);
        let height = |route: &FumeRoute, x: f64| points(route).into_iter().min_by(|one, two| (one.0 - x).abs().total_cmp(&(two.0 - x).abs())).map(|point| point.1);
        let middle = layout.cleft().0;

        assert!(to_right.at(0.0).0 < to_left.at(0.0).0 && to_right.at(1.0).0 > to_left.at(1.0).0);
        assert!(height(&to_right, middle).is_some() && height(&to_left, middle).is_some());
    }

    #[test]
    fn no_route_passes_through_another_jar() {
        let layout = FumeLayout::new(1200.0, 800.0);
        for slot in 0..3 {
            for route in [-0.5, 0.5].into_iter().flat_map(|lean| routes(&layout, slot, lean)) {
                let points = points(&route);
                for other in (0..3).filter(|other| *other != slot) {
                    let outline = jar(&layout, other).outline();
                    assert!(points.iter().all(|point| !outline.contains(point.0, point.1)), "slot {slot} hits {other}");
                }
            }
        }
    }

    #[test]
    fn the_bowl_stands_on_the_rock_beside_the_cleft_below_the_jars() {
        for (width, height) in [(1200.0, 800.0), (800.0, 900.0), (600.0, 900.0), (1600.0, 700.0)] {
            let layout = FumeLayout::new(width, height);
            let bowl = layout.bowl();
            let (cleft, floor) = layout.cleft();

            let (left, _, wide, _) = bowl.picture();
            assert!(bowl.centre - bowl.half_width > cleft + layout.cleft_span() * 0.5, "{width}x{height}");
            assert!(left > cleft && left + wide < width, "{width}x{height}");
            assert!(bowl.base < floor && bowl.base > floor - height * 0.05);
            assert!(bowl.rim > jar(&layout, 2).mouth().1 + height * 0.1, "{width}x{height}");
        }
    }

    #[test]
    fn every_pour_leaves_its_mouth_heading_down_and_drops_into_the_bowl_from_above() {
        let layout = FumeLayout::new(1200.0, 800.0);
        let bowl = layout.bowl();
        for slot in 0..3 {
            let jar = jar(&layout, slot);
            let pour = layout.pour(&jar, &bowl);
            let points = points(&pour);
            let (first, last) = (points[8], points[points.len() - 9]);
            let end = pour.at(1.0);

            assert!(jar.outline().contains(pour.at(0.0).0, pour.at(0.0).1));
            assert!(first.1 > pour.at(0.0).1 && (first.0 - jar.centre).abs() < 2.0);
            assert!(last.1 < end.1 && (last.0 - bowl.centre).abs() < 2.0);
            assert!(bowl.contains(end.0, end.1));
            assert!(points.windows(2).all(|pair| pair[1].1 >= pair[0].1 - 1e-6), "slot {slot} climbs");
        }
    }

    #[test]
    fn no_pour_passes_through_another_jar() {
        let layout = FumeLayout::new(1200.0, 800.0);
        let bowl = layout.bowl();
        for slot in 0..3 {
            let points = points(&layout.pour(&jar(&layout, slot), &bowl));
            for other in (0..3).filter(|other| *other != slot) {
                let outline = jar(&layout, other).outline();
                assert!(points.iter().all(|point| !outline.contains(point.0, point.1)), "slot {slot} hits {other}");
            }
        }
    }

    #[test]
    fn the_unit_follows_the_smaller_side() {
        assert!((FumeLayout::new(2400.0, 800.0).unit() - 1.0).abs() < 1e-9);
        assert!((FumeLayout::new(600.0, 800.0).unit() - 0.5).abs() < 1e-9);
    }
}
