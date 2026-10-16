use crate::{BowlBrew, FumeFate, FumeFeed, FumeJar, FumeLayout, FumePuff, FumeStream, FumeWisp, SplitMix};

#[derive(Debug, Clone)]
pub struct FumeScene {
    layout: FumeLayout,
    jars: Vec<FumeJar>,
    wisps: Vec<FumeWisp>,
    feed: FumeFeed,
    random: SplitMix,
    now: f64,
    scattered: usize,
    streams: Vec<(usize, FumeStream, f64)>,
    brew: BowlBrew,
    harvests: usize,
}

impl FumeScene {
    const LONGEST_STEP: f64 = 0.25;
    const STAGGER: f64 = 0.45;

    #[must_use]
    pub fn new(width: f64, height: f64, seed: u64) -> Self {
        let layout = FumeLayout::new(width, height);
        let (jar_height, jar_width) = layout.jar_size();
        let jars = FumeFate::JARS
            .iter()
            .enumerate()
            .map(|(slot, fate)| FumeJar::new(*fate, layout.jar_centre(slot), layout.hang(), jar_height, jar_width))
            .collect();
        Self {
            layout,
            jars,
            wisps: Vec::new(),
            feed: FumeFeed::new(seed),
            random: SplitMix::new(seed ^ 0xf00d),
            now: 0.0,
            scattered: 0,
            streams: Vec::new(),
            brew: BowlBrew::new(seed ^ 0xb0e1),
            harvests: 0,
        }
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        if (width - self.layout.width).abs() < 0.5 && (height - self.layout.height).abs() < 0.5 {
            return;
        }
        self.layout = FumeLayout::new(width, height);
        let (jar_height, jar_width) = self.layout.jar_size();
        let hang = self.layout.hang();
        for (slot, jar) in self.jars.iter_mut().enumerate() {
            *jar = jar.placed(self.layout.jar_centre(slot), hang, jar_height, jar_width);
        }
        self.wisps.clear();
        let bowl = self.layout.bowl();
        for (slot, stream, _) in &mut self.streams {
            *stream = stream.rerouted(self.layout.pour(&self.jars[*slot], &bowl), self.layout.unit());
        }
    }

    pub fn harvest(&mut self) -> bool {
        let mut started = false;
        let bowl = self.layout.bowl();
        let mut order: Vec<usize> = (0..self.jars.len()).collect();
        order.sort_by(|one, two| (bowl.centre - self.jars[*one].centre).abs().total_cmp(&(bowl.centre - self.jars[*two].centre).abs()));
        let mut delay = 0.0;
        for slot in order {
            let jar = &mut self.jars[slot];
            let Some(drain) = jar.harvest(delay) else {
                continue;
            };
            let route = self.layout.pour(jar, &bowl);
            let colour = jar.fate.colour();
            let seed = self.random.next_u64();
            self.streams.push((slot, FumeStream::new(colour, route, drain, self.layout.unit(), seed), 0.0));
            delay += Self::STAGGER;
            started = true;
        }
        self.harvests += usize::from(started);
        started
    }

    pub fn advance(&mut self, seconds: f64) {
        let step = seconds.clamp(0.0, Self::LONGEST_STEP);
        self.now += step;
        for stage in self.feed.advance(step) {
            self.release(FumeFate::of(stage));
        }
        let now = self.now;
        for wisp in &self.wisps {
            if let Some(slot) = wisp.fate.jar() {
                if wisp.arrived(now) && !wisp.arrived(now - step) {
                    self.jars[slot].collect();
                }
            }
            if wisp.broken_for(now).is_some() && wisp.broken_for(now - step).is_none() {
                self.scattered += 1;
            }
        }
        self.wisps.retain(|wisp| !wisp.finished(now));
        for jar in &mut self.jars {
            jar.advance(step);
        }
        for (_, stream, poured) in &mut self.streams {
            let landed = stream.landed(now).max(*poured);
            self.brew.pour(stream.colour, landed - *poured);
            *poured = landed;
        }
        self.streams.retain(|(_, stream, poured)| now < stream.last_landing() || *poured < stream.amount());
        self.brew.advance(step);
    }

    #[must_use]
    pub const fn layout(&self) -> &FumeLayout {
        &self.layout
    }

    #[must_use]
    pub fn jars(&self) -> &[FumeJar] {
        &self.jars
    }

    #[must_use]
    pub const fn now(&self) -> f64 {
        self.now
    }

    #[must_use]
    pub const fn scattered(&self) -> usize {
        self.scattered
    }

    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.wisps.len()
    }

    #[must_use]
    pub fn puffs(&self) -> Vec<FumePuff> {
        self.wisps.iter().flat_map(|wisp| wisp.puffs(self.now)).collect()
    }

    #[must_use]
    pub fn pouring(&self) -> Vec<FumePuff> {
        self.streams.iter().flat_map(|(_, stream, _)| stream.puffs(self.now)).collect()
    }

    #[must_use]
    pub fn smoke(&self) -> Vec<FumePuff> {
        let bowl = self.layout.bowl();
        self.brew.smoke((bowl.centre, bowl.rim), bowl.half_width, self.layout.unit())
    }

    #[must_use]
    pub const fn brew(&self) -> &BowlBrew {
        &self.brew
    }

    #[must_use]
    pub const fn harvests(&self) -> usize {
        self.harvests
    }

    fn release(&mut self, fate: FumeFate) {
        let lean = self.random.unit() - 0.5;
        let origin = self.layout.source_anywhere(self.random.unit());
        let slot = fate.jar().unwrap_or_else(|| (self.random.unit() * self.jars.len() as f64) as usize % self.jars.len());
        let route = self.layout.route(origin, &self.jars[slot], lean);
        let seed = self.random.next_u64();
        self.wisps.push(FumeWisp::new(fate, route, self.now, self.layout.unit(), seed));
    }
}

#[cfg(test)]
mod tests {
    use super::FumeScene;
    use crate::{FumeFate, FumeFeed};

    fn run(scene: &mut FumeScene, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            scene.advance(1.0 / 60.0);
        }
    }

    #[test]
    fn over_a_minute_the_jars_fill_and_rejected_fumes_are_puffed_away() {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        run(&mut scene, 60.0);
        let collected: usize = scene.jars().iter().map(|jar| jar.collected()).sum();
        let arrived = FumeFeed::RATE * 60.0;

        assert!(scene.jars().iter().all(|jar| jar.collected() > 0));
        assert!(scene.scattered() > 0);
        assert!(((collected + scene.scattered()) as f64) > arrived * 0.85, "{collected} {}", scene.scattered());
        assert!(scene.jars()[0].collected() > scene.jars()[2].collected());
    }

    #[test]
    fn finished_wisps_are_dropped_so_the_scene_stays_small() {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        run(&mut scene, 120.0);

        assert!(scene.in_flight() < 40, "{}", scene.in_flight());
    }

    #[test]
    fn a_long_pause_does_not_release_a_burst() {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        scene.advance(3600.0);

        assert!(scene.in_flight() < 10);
    }

    #[test]
    fn resizing_keeps_the_counts() {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        run(&mut scene, 30.0);
        let before: Vec<usize> = scene.jars().iter().map(|jar| jar.collected()).collect();
        scene.resize(800.0, 600.0);

        assert_eq!(scene.jars().iter().map(|jar| jar.collected()).collect::<Vec<_>>(), before);
        assert_eq!(scene.jars()[1].fate, FumeFate::Round);
    }

    fn filled() -> FumeScene {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        run(&mut scene, 60.0);
        scene
    }

    #[test]
    fn a_harvest_empties_every_jar_into_the_bowl_one_after_another() {
        let mut scene = filled();
        let held: f64 = scene.jars().iter().map(|jar| jar.fullness()).sum();
        let counts: Vec<usize> = scene.jars().iter().map(|jar| jar.collected()).collect();

        assert!(scene.harvest());
        run(&mut scene, 1.2);
        let emptied: Vec<f64> = scene.jars().iter().map(|jar| jar.emptied()).collect();
        run(&mut scene, 6.0);

        assert!(emptied[2] > emptied[1] && emptied[1] > emptied[0], "{emptied:?}");
        assert!(scene.jars().iter().all(|jar| jar.fullness() < 0.15), "{:?}", scene.jars().iter().map(|jar| jar.fullness()).collect::<Vec<_>>());
        assert!(scene.brew().charge() > held * 0.6, "{} of {held}", scene.brew().charge());
        assert!(scene.jars().iter().zip(&counts).all(|(jar, before)| jar.collected() >= *before));
        assert_eq!(scene.harvests(), 1);
    }

    #[test]
    fn every_drop_that_leaves_a_jar_reaches_the_bowl() {
        let mut scene = filled();
        let held: f64 = scene.jars().iter().map(|jar| jar.fullness()).sum();
        let before = scene.brew().received();
        scene.harvest();
        run(&mut scene, 10.0);

        assert!((scene.brew().received() - before - held).abs() < 1e-9, "{} vs {held}", scene.brew().received() - before);
        assert!(scene.streams.is_empty());
    }

    #[test]
    fn while_pouring_the_scene_shows_streams_and_then_smoke_rises_from_the_bowl() {
        let mut scene = filled();
        scene.harvest();
        run(&mut scene, 1.5);
        let bowl = scene.layout().bowl();

        assert!(scene.pouring().len() > 30);
        run(&mut scene, 6.0);
        assert!(scene.pouring().is_empty());
        assert!(scene.smoke().iter().any(|puff| puff.y < bowl.rim - 40.0));
        assert!(scene.brew().glow() > 0.1);
    }

    #[test]
    fn harvesting_empty_jars_or_twice_at_once_does_nothing_more() {
        let mut scene = FumeScene::new(1200.0, 800.0, 4);
        assert!(!scene.harvest());
        let mut scene = filled();

        assert!(scene.harvest());
        assert!(!scene.harvest());
        assert_eq!(scene.harvests(), 1);
    }

    #[test]
    fn the_jars_refill_after_a_harvest() {
        let mut scene = filled();
        scene.harvest();
        run(&mut scene, 40.0);

        assert!(scene.jars()[0].fullness() > 0.2);
        assert!(scene.harvest());
    }

    #[test]
    fn resizing_mid_pour_keeps_the_streams_flowing_into_the_new_bowl() {
        let mut scene = filled();
        scene.harvest();
        run(&mut scene, 1.0);
        scene.resize(900.0, 700.0);
        run(&mut scene, 1.0);
        let bowl = scene.layout().bowl();

        assert!(scene.pouring().iter().all(|puff| puff.x < 900.0 + 60.0 && puff.y < bowl.base + 40.0));
        assert!(scene.streams.iter().all(|(_, stream, _)| bowl.contains(stream.end().0, stream.end().1)));
    }

    #[test]
    fn puffs_stay_on_the_page() {
        let mut scene = FumeScene::new(1200.0, 800.0, 8);
        for _ in 0..1800 {
            scene.advance(1.0 / 60.0);
            assert!(scene.puffs().iter().all(|puff| puff.x > -80.0 && puff.x < 1280.0 && puff.y > -80.0 && puff.y < 880.0));
        }
    }
}
