use crate::{BowlBrew, FumeBowl, FumeJar, FumeLayout, FumeMark, FumePolygon, FumeScene, GasCloud, HexColor, ValueNoise, VesselMask};

#[derive(Debug, Clone, PartialEq)]
pub struct FumeSketch {
    pub background: HexColor,
    pub marks: Vec<FumeMark>,
}

impl FumeSketch {
    pub const NIGHT: HexColor = HexColor::rgb([0x0c, 0x0a, 0x09]);
    const ROCK: HexColor = HexColor::rgb([0x22, 0x1d, 0x1a]);
    const CHASM: HexColor = HexColor::rgb([0x2a, 0x1d, 0x45]);
    const DEPTHS: HexColor = HexColor::rgb([0x4a, 0x2d, 0x6b]);
    const CLAY: HexColor = HexColor::rgb([0x3a, 0x31, 0x2b]);
    const CORD: HexColor = HexColor::rgb([0x6b, 0x5e, 0x50]);
    const CRAG_STEPS: usize = 48;
    pub const VESSEL: &'static str = "fumes/glass-flask.png";
    pub const VESSEL_MASK: &'static str = "fumes/glass-flask-mask.png";

    #[must_use]
    pub fn of(scene: &FumeScene) -> Self {
        let mut marks = Self::ground(scene);
        for puff in scene.puffs() {
            marks.push(FumeMark::Glow { x: puff.x, y: puff.y, radius: puff.radius, colour: puff.colour, alpha: puff.alpha * 0.55 });
        }
        marks.extend(Self::beam(scene.layout(), scene.jars()));
        for jar in scene.jars() {
            marks.extend(Self::jar(jar, scene.now()));
        }
        for puff in scene.pouring() {
            marks.push(FumeMark::Glow { x: puff.x, y: puff.y, radius: puff.radius, colour: puff.colour, alpha: puff.alpha * 0.55 });
        }
        marks.extend(Self::bowl(&scene.layout().bowl(), scene.brew()));
        for puff in scene.smoke() {
            marks.push(FumeMark::Glow { x: puff.x, y: puff.y, radius: puff.radius, colour: puff.colour, alpha: puff.alpha * 0.55 });
        }
        Self { background: Self::NIGHT, marks }
    }

    fn ground(scene: &FumeScene) -> Vec<FumeMark> {
        let layout = scene.layout();
        let (centre, floor) = layout.cleft();
        let span = layout.cleft_span();
        let noise = ValueNoise::new(41);
        let mut rock: Vec<(f64, f64)> = vec![(0.0, layout.height)];
        let mut crack: Vec<(f64, f64)> = Vec::new();
        for step in 0..=Self::CRAG_STEPS {
            let x = layout.width * step as f64 / Self::CRAG_STEPS as f64;
            let rough = noise.fractal(x / (60.0 * layout.unit()), 0.5, 3) * 10.0 * layout.unit();
            let gap = ((x - centre).abs() / (span * 0.5)).min(1.0);
            let dip = (1.0 - gap * gap) * 16.0 * layout.unit();
            rock.push((x, floor - 14.0 * layout.unit() + rough + dip));
        }
        rock.push((layout.width, layout.height));
        for step in 0..=Self::CRAG_STEPS {
            let across = step as f64 / Self::CRAG_STEPS as f64;
            let x = centre + (across - 0.5) * span;
            let jag = noise.fractal(across * 9.0, 4.2, 2) * 6.0 * layout.unit();
            crack.push((x, floor - 2.0 * layout.unit() + jag * 0.4));
        }
        for step in (0..=Self::CRAG_STEPS).rev() {
            let across = step as f64 / Self::CRAG_STEPS as f64;
            let x = centre + (across - 0.5) * span;
            let depth = (std::f64::consts::PI * across).sin().powf(0.7) * 34.0 * layout.unit();
            let jag = noise.fractal(across * 11.0, 9.1, 2) * 5.0 * layout.unit();
            crack.push((x, floor + depth + jag));
        }
        vec![
            FumeMark::Glow { x: centre, y: floor, radius: span * 0.9, colour: Self::CHASM, alpha: 0.9 },
            FumeMark::Fill { shape: FumePolygon::new(rock), colour: Self::ROCK, alpha: 1.0, below: None },
            FumeMark::Fill { shape: FumePolygon::new(crack), colour: Self::DEPTHS, alpha: 1.0, below: None },
        ]
    }

    fn beam(layout: &FumeLayout, jars: &[FumeJar]) -> Vec<FumeMark> {
        let unit = layout.unit();
        let (from, to, beam) = layout.beam();
        let thick = 10.0 * unit;
        let lintel = FumePolygon::new(vec![(from, beam - thick), (to, beam - thick), (to - 6.0 * unit, beam), (from + 6.0 * unit, beam)]);
        let mut marks = vec![FumeMark::Fill { shape: lintel, colour: Self::CLAY, alpha: 1.0, below: None }];
        for jar in jars {
            let (half, end) = (unit, jar.top() + jar.height * 0.03);
            let cord = FumePolygon::new(vec![(jar.centre - half, beam), (jar.centre + half, beam), (jar.centre + half, end), (jar.centre - half, end)]);
            marks.push(FumeMark::Fill { shape: cord, colour: Self::CORD, alpha: 1.0, below: None });
        }
        marks
    }

    fn bowl(bowl: &FumeBowl, brew: &BowlBrew) -> Vec<FumeMark> {
        let glow = brew.glow();
        let (left, top, width, height) = bowl.picture();
        let mut marks = vec![FumeMark::Picture { source: FumeBowl::PICTURE, left, top, width, height, alpha: 1.0 }];
        let charge = brew.charge().min(1.0).sqrt();
        if charge > 0.0 {
            marks.push(FumeMark::Glow { x: bowl.centre, y: bowl.rim, radius: bowl.half_width * 1.1, colour: brew.colour(), alpha: (0.25 + 0.5 * glow.min(1.0)) * charge });
        }
        if glow > 0.0 {
            marks.push(FumeMark::Glow { x: bowl.centre, y: bowl.rim, radius: bowl.half_width * (1.6 + 1.2 * glow), colour: brew.colour(), alpha: 0.35 * glow });
        }
        marks
    }

    fn jar(jar: &FumeJar, now: f64) -> Vec<FumeMark> {
        vec![
            FumeMark::Glow { x: jar.centre, y: jar.top() + jar.height * 0.4, radius: jar.width * 0.8, colour: jar.fate.colour(), alpha: 0.03 + 0.1 * jar.fullness() },
            FumeMark::Gas {
                cloud: GasCloud::of(jar, now),
                mask: VesselMask { source: Self::VESSEL_MASK, left: jar.left(), top: jar.top(), width: jar.width, height: jar.height },
            },
            FumeMark::Picture { source: Self::VESSEL, left: jar.left(), top: jar.top(), width: jar.width, height: jar.height, alpha: 1.0 },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::FumeSketch;
    use crate::{FumeMark, FumeScene};

    #[test]
    fn an_empty_scene_draws_the_ground_and_three_empty_jars() {
        let sketch = FumeSketch::of(&FumeScene::new(1200.0, 800.0, 1));
        let pictures = sketch.marks.iter().filter(|mark| matches!(mark, FumeMark::Picture { .. })).count();
        let glows = sketch.marks.iter().filter(|mark| matches!(mark, FumeMark::Glow { .. })).count();

        assert_eq!(pictures, 3 + 1);
        assert_eq!(glows, 4);
        assert_eq!(sketch.marks.iter().filter(|mark| matches!(mark, FumeMark::Fill { .. })).count(), 3 + 3);
        assert!(sketch.marks.iter().all(|mark| mark.coverage(-5000.0, -5000.0).abs() < 1e-9));
    }

    #[test]
    fn a_running_scene_draws_its_fumes_as_glows() {
        let mut scene = FumeScene::new(1200.0, 800.0, 1);
        for _ in 0..300 {
            scene.advance(1.0 / 60.0);
        }
        let empty = FumeSketch::of(&FumeScene::new(1200.0, 800.0, 1)).marks.len();

        assert!(FumeSketch::of(&scene).marks.len() > empty);
    }

    #[test]
    fn a_harvest_lights_the_bowl_with_the_colour_poured_into_it() {
        let mut scene = FumeScene::new(1200.0, 800.0, 1);
        for _ in 0..3600 {
            scene.advance(1.0 / 60.0);
        }
        let bowl = scene.layout().bowl();
        let ember = |scene: &FumeScene| FumeSketch::of(scene).marks.iter().fold([0.0; 3], |under, mark| mark.paint(under, bowl.centre, bowl.rim));
        let before = ember(&scene);
        scene.harvest();
        for _ in 0..360 {
            scene.advance(1.0 / 60.0);
        }
        let after = ember(&scene);

        assert!(after.iter().sum::<f64>() > before.iter().sum::<f64>() + 60.0, "{before:?} {after:?}");
    }
}
