use crate::{FumeBowl, FumeMark, FumePolygon, HexColor, Limestone, OracleScene, StoneInscription, StoneSketch, ValueNoise};

#[derive(Debug, Clone, PartialEq)]
pub struct OracleSketch {
    pub background: HexColor,
    pub under: Vec<FumeMark>,
    pub stones: Vec<StoneSketch>,
    pub front: Vec<FumeMark>,
    pub over: Vec<FumeMark>,
}

impl OracleSketch {
    pub const NIGHT: HexColor = HexColor::rgb([0x0e, 0x0b, 0x09]);
    const DUSK: HexColor = HexColor::rgb([0x5a, 0x3c, 0x22]);
    const ROCK: HexColor = HexColor::rgb([0x1d, 0x18, 0x14]);
    const SUN: HexColor = HexColor::rgb([0xff, 0xe2, 0xa8]);
    const STRIKE: HexColor = HexColor::rgb([0xff, 0xb8, 0x5a]);
    const SHADE: HexColor = HexColor::rgb([0x05, 0x04, 0x03]);
    const AGEING: f64 = 0.035;
    const OLDEST: f64 = 0.4;
    const DROP: f64 = 0.18;
    const BEAM_ALPHA: f64 = 0.16;
    const BEAM_LAYERS: usize = 12;
    const GROUND_STEPS: usize = 40;
    const CONTACT_LAYERS: usize = 6;
    const CONTACT_DEPTH: f64 = 14.0;
    const CONTACT_ALPHA: f64 = 0.12;

    #[must_use]
    pub fn of(scene: &OracleScene) -> Self {
        let layout = scene.layout();
        let arrival = scene.arrival();
        let rest = scene.wall().ground_course() as f64;
        let stones = scene.wall().stones(layout, rest);
        let pan = (scene.view() - rest) * layout.course_height();
        let blocks = scene.wall().blocks();
        let newest = blocks.len().saturating_sub(1);
        let mut sketches = Vec::with_capacity(stones.len());
        for (at, (stone, block)) in stones.iter().zip(blocks).enumerate() {
            let inscription = StoneInscription::new(stone, block);
            let fresh = at == newest && !arrival.done();
            let progress = if fresh { arrival.carve() } else { 1.0 };
            let carvings = inscription.cut(progress);
            let letters = (inscription.letters() as f64 * progress).round() as usize;
            let settle = if fresh { arrival.settle() } else { 1.0 };
            let sketch = StoneSketch {
                key: block.hash.clone(),
                stone: stone.clone(),
                offset: (0.0, pan - (1.0 - settle) * layout.course_height() * Self::DROP),
                alpha: settle,
                shade: ((newest - at) as f64 * Self::AGEING).min(Self::OLDEST),
                patch: Limestone::patch(u64::from(block.height)),
                tone: Limestone::tone(u64::from(block.height)),
                tint: Limestone::tint(u64::from(block.height)),
                light: layout.light(),
                carvings,
                letters,
            };
            sketches.push(sketch);
        }
        let over = Self::beam(scene, sketches.last());
        Self { background: Self::NIGHT, under: Self::sky(scene), stones: sketches, front: Self::ground(scene), over }
    }

    fn sky(scene: &OracleScene) -> Vec<FumeMark> {
        let layout = scene.layout();
        let (sun_x, sun_y) = layout.sun();
        vec![FumeMark::Glow { x: sun_x, y: sun_y, radius: layout.width.max(layout.height) * 0.9, colour: Self::DUSK, alpha: 0.55 }]
    }

    fn ground(scene: &OracleScene) -> Vec<FumeMark> {
        let layout = scene.layout();
        let unit = layout.unit();
        let noise = ValueNoise::new(17);
        let mut rock = vec![(0.0, layout.height)];
        for step in 0..=Self::GROUND_STEPS {
            let x = layout.width * step as f64 / Self::GROUND_STEPS as f64;
            rock.push((x, layout.ground() - 2.0 * unit + noise.fractal(x / (70.0 * unit), 0.3, 3) * 5.0 * unit));
        }
        rock.push((layout.width, layout.height));
        let tripod = layout.tripod();
        let (left, top, width, height) = tripod.picture();
        let mut marks = vec![FumeMark::Fill { shape: FumePolygon::new(rock), colour: Self::ROCK, alpha: 1.0, below: None }];
        let wall = layout.wall();
        for layer in 1..=Self::CONTACT_LAYERS {
            let depth = Self::CONTACT_DEPTH * unit * layer as f64 / Self::CONTACT_LAYERS as f64;
            let spread = depth * 1.5;
            let band = FumePolygon::new(vec![(wall.left - spread, wall.bottom - 4.0 * unit), (wall.right + spread, wall.bottom - 4.0 * unit), (wall.right + spread * 0.5, wall.bottom + depth), (wall.left - spread * 0.5, wall.bottom + depth)]);
            marks.push(FumeMark::Fill { shape: band, colour: Self::SHADE, alpha: Self::CONTACT_ALPHA, below: None });
        }
        let foot = FumePolygon::new(vec![(left + width * 0.1, tripod.base - unit), (left + width * 1.1, tripod.base - unit), (left + width * 1.3, tripod.base + 6.0 * unit), (left + width * 0.2, tripod.base + 6.0 * unit)]);
        marks.push(FumeMark::Fill { shape: foot, colour: Self::SHADE, alpha: 0.4, below: None });
        marks.push(FumeMark::Picture { source: FumeBowl::PICTURE, left, top, width, height, alpha: 1.0 });
        marks
    }

    fn beam(scene: &OracleScene, newest: Option<&StoneSketch>) -> Vec<FumeMark> {
        let arrival = scene.arrival();
        let strength = arrival.beam();
        if strength <= 0.0 {
            return Vec::new();
        }
        let layout = scene.layout();
        let unit = layout.unit();
        let tripod = layout.tripod();
        let bowl = (tripod.centre, tripod.rim);
        let stone = newest.map_or(bowl, |sketch| sketch.stone.centre());
        let cross = arrival.cross();
        let target = (bowl.0 + (stone.0 - bowl.0) * cross, bowl.1 + (stone.1 - bowl.1) * cross);
        let sun = layout.sun();
        let reach = arrival.strike();
        let end = (sun.0 + (target.0 - sun.0) * reach, sun.1 + (target.1 - sun.1) * reach);
        let (dx, dy) = (end.0 - sun.0, end.1 - sun.1);
        let length = dx.hypot(dy).max(1.0);
        let (nx, ny) = (-dy / length, dx / length);
        let (near, far) = (10.0 * unit, 30.0 * unit + 50.0 * unit * cross);
        let mut marks: Vec<FumeMark> = (1..=Self::BEAM_LAYERS)
            .map(|layer| {
                let share = layer as f64 / Self::BEAM_LAYERS as f64;
                let (near, far) = (near * share, far * share);
                let shaft = FumePolygon::new(vec![
                    (sun.0 + nx * near, sun.1 + ny * near),
                    (end.0 + nx * far, end.1 + ny * far),
                    (end.0 - nx * far, end.1 - ny * far),
                    (sun.0 - nx * near, sun.1 - ny * near),
                ]);
                FumeMark::Fill { shape: shaft, colour: Self::SUN, alpha: Self::BEAM_ALPHA * strength / Self::BEAM_LAYERS as f64 * 2.0, below: None }
            })
            .collect();
        let bowl_glow = strength * (1.0 - cross * 0.6) * reach * 0.6;
        marks.push(FumeMark::Glow { x: bowl.0, y: bowl.1, radius: tripod.half_width * 1.6, colour: Self::STRIKE, alpha: bowl_glow });
        marks
    }

}

#[cfg(test)]
mod tests {
    use super::OracleSketch;
    use crate::{FumeMark, OracleScene};

    fn after(seconds: f64) -> OracleScene {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        let mut clock = 0.0;
        while clock < seconds {
            scene.advance(1.0 / 30.0);
            clock += 1.0 / 30.0;
        }
        scene
    }

    fn until_arrival() -> OracleScene {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        while !scene.advance(1.0 / 30.0) {}
        scene
    }

    #[test]
    fn a_resting_wall_shows_every_stone_fully_cut_in_place_and_no_beam() {
        let scene = after(0.5);
        let sketch = OracleSketch::of(&scene);

        assert_eq!(sketch.stones.len(), scene.wall().blocks().len());
        assert!(sketch.stones.iter().all(|stone| stone.offset.0 == 0.0 && stone.offset.1.abs() < 1e-9 && (stone.alpha - 1.0).abs() < 1e-9 && !stone.carvings.is_empty()));
        assert_eq!(sketch.over.iter().filter(|mark| matches!(mark, FumeMark::Glow { .. })).count(), 0);
    }

    #[test]
    fn older_stones_are_darker_and_the_newest_is_clean() {
        let sketch = OracleSketch::of(&after(0.5));

        assert!(sketch.stones.last().is_some_and(|stone| stone.shade.abs() < 1e-9));
        assert!(sketch.stones.windows(2).all(|pair| pair[0].shade >= pair[1].shade));
        assert!(sketch.stones[0].shade > 0.2);
    }

    #[test]
    fn a_new_block_drops_in_uncut_under_a_beam_that_lights_the_tripod() {
        let mut scene = until_arrival();
        scene.advance(1.0);
        let sketch = OracleSketch::of(&scene);
        let newest = sketch.stones.last().expect("stone");

        assert!(newest.offset.1 < 0.0 || newest.alpha < 1.0 || newest.carvings.is_empty());
        assert!(newest.carvings.is_empty());
        let tripod = scene.layout().tripod();
        assert!(sketch.over.iter().any(|mark| matches!(mark, FumeMark::Glow { x, y, alpha, .. } if (*x - tripod.centre).abs() < 1e-9 && (*y - tripod.rim).abs() < 1e-9 && *alpha > 0.1)));
    }

    #[test]
    fn halfway_through_the_carving_only_part_of_the_inscription_is_cut() {
        let mut scene = until_arrival();
        scene.advance(5.6);
        let sketch = OracleSketch::of(&scene);
        let newest = sketch.stones.last().expect("stone");
        let older = &sketch.stones[sketch.stones.len() - 2];

        assert!(newest.offset.1.abs() < 1e-9);
        assert!(newest.letters > 0 && newest.letters < older.letters + 6);
        assert!(!newest.carvings.is_empty());
    }

    #[test]
    fn the_beam_is_built_of_nested_layers_so_its_edge_is_soft() {
        let mut scene = until_arrival();
        scene.advance(2.0);
        let sketch = OracleSketch::of(&scene);
        let shafts: Vec<f64> = sketch.over.iter().filter_map(|mark| if let FumeMark::Fill { shape, .. } = mark { Some(shape.area()) } else { None }).collect();

        assert!(shafts.len() >= 8);
        assert!(shafts.windows(2).all(|pair| pair[1] > pair[0]));
    }

    #[test]
    fn nothing_but_the_beam_is_drawn_over_the_wall() {
        let sketch = OracleSketch::of(&after(0.5));

        assert!(sketch.over.is_empty());
    }

    #[test]
    fn the_ground_and_tripod_stand_in_front_of_the_wall_so_a_sinking_course_disappears_behind_them() {
        let sketch = OracleSketch::of(&after(0.5));

        assert!(sketch.front.iter().any(|mark| matches!(mark, FumeMark::Picture { .. })));
        assert!(sketch.under.iter().all(|mark| matches!(mark, FumeMark::Glow { .. })));
    }

    #[test]
    fn during_a_pan_every_stone_slides_by_the_same_amount_and_keeps_its_cache_key() {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        let start = scene.view();
        while (scene.view() - start).abs() < 0.3 {
            scene.advance(1.0 / 30.0);
        }
        let early = OracleSketch::of(&scene);
        scene.advance(0.2);
        let later = OracleSketch::of(&scene);
        let old = &early.stones[..early.stones.len() - 1];

        assert!(old.iter().all(|stone| (stone.offset.1 - old[0].offset.1).abs() < 1e-9 && stone.offset.1 < 0.0));
        let key = |sketch: &OracleSketch, at: usize| sketch.stones[at].cache_key(2.0);
        assert_eq!(key(&early, early.stones.len() - 2), key(&later, later.stones.len() - 2));
    }
}
