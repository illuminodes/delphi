use crate::{CarveBox, FumeBowl, WallLight};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OracleLayout {
    pub width: f64,
    pub height: f64,
}

impl OracleLayout {
    pub const COURSES: usize = 3;
    pub const PER_COURSE: f64 = 4.0;
    const DESIGN: (f64, f64) = (1200.0, 800.0);
    const GROUND: f64 = 0.9;
    const WALL_TOP: f64 = 0.14;
    const WALL_LEFT: f64 = 0.25;
    const WALL_RIGHT: f64 = 0.0;
    const TRIPOD_X: f64 = 0.14;
    const TRIPOD_HEIGHT: f64 = 0.34;
    const SUN: (f64, f64) = (0.03, -0.08);

    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn unit(&self) -> f64 {
        (self.width / Self::DESIGN.0).min(self.height / Self::DESIGN.1).max(0.25)
    }

    #[must_use]
    pub fn ground(&self) -> f64 {
        self.height * Self::GROUND
    }

    #[must_use]
    pub fn wall(&self) -> CarveBox {
        CarveBox::new(self.width * Self::WALL_LEFT, self.height * Self::WALL_TOP, self.width * (1.0 - Self::WALL_RIGHT), self.ground())
    }

    #[must_use]
    pub fn course_height(&self) -> f64 {
        self.wall().height() / Self::COURSES as f64
    }

    #[must_use]
    pub fn span(&self, fullness: f64) -> f64 {
        self.wall().width() / Self::PER_COURSE * (0.35 + 0.65 * fullness.clamp(0.0, 1.0))
    }

    #[must_use]
    pub fn tripod(&self) -> FumeBowl {
        let height = (self.height * Self::TRIPOD_HEIGHT).min(self.wall().left * 0.8 / FumeBowl::reach(1.0) * 0.5);
        FumeBowl::standing(self.width * Self::TRIPOD_X, self.ground(), height)
    }

    #[must_use]
    pub fn sun(&self) -> (f64, f64) {
        (self.width * Self::SUN.0, self.height * Self::SUN.1)
    }

    #[must_use]
    pub fn light(&self) -> WallLight {
        WallLight::new(self.sun(), self.width.hypot(self.height))
    }

}

#[cfg(test)]
mod tests {
    use super::OracleLayout;

    #[test]
    fn the_tripod_stands_on_the_ground_left_of_the_wall() {
        for (width, height) in [(1200.0, 800.0), (900.0, 900.0), (1600.0, 700.0)] {
            let layout = OracleLayout::new(width, height);
            let tripod = layout.tripod();
            let (left, top, wide, _) = tripod.picture();

            assert!(left > 0.0 && left + wide < layout.wall().left, "{width}x{height}");
            assert!((tripod.base - layout.ground()).abs() < 1e-9 && top > 0.0);
        }
    }

    #[test]
    fn the_wall_holds_three_courses_of_about_four_full_stones_each() {
        let layout = OracleLayout::new(1200.0, 800.0);

        assert!((layout.course_height() * 3.0 - layout.wall().height()).abs() < 1e-9);
        assert!((layout.span(1.0) * 4.0 - layout.wall().width()).abs() < 1e-9);
        assert!(layout.span(0.2) < layout.span(0.9));
    }

    #[test]
    fn the_sun_is_above_the_scene_and_the_wall_runs_off_the_right_edge() {
        let layout = OracleLayout::new(1200.0, 800.0);

        assert!(layout.sun().1 < 0.0);
        assert!((layout.wall().right - layout.width).abs() < 1e-9);
    }
}
