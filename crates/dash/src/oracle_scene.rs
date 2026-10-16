use crate::{BlockArrival, BlockSimulator, ChainBlock, OracleLayout, OracleWall, SplitMix, WallPan};

#[derive(Debug, Clone)]
pub struct OracleScene {
    layout: OracleLayout,
    wall: OracleWall,
    pan: WallPan,
    arrival: BlockArrival,
    feed: Option<BlockSimulator>,
    random: SplitMix,
    seed: u64,
    now: f64,
    since: f64,
    next_in: f64,
}

impl OracleScene {
    pub const TARGET: f64 = 600.0;
    pub const PACE: f64 = 24.0;
    const PREFILL: usize = 11;
    const SHORTEST_WAIT: f64 = 12.0;

    #[must_use]
    pub fn simulated(width: f64, height: f64, seed: u64) -> Self {
        let mut scene = Self::waiting(width, height, seed);
        let mut feed = BlockSimulator::new(seed);
        for _ in 0..Self::PREFILL {
            scene.settle(feed.next_block());
        }
        scene.feed = Some(feed);
        scene.next_in = Self::SHORTEST_WAIT * 0.5;
        scene
    }

    #[must_use]
    pub fn waiting(width: f64, height: f64, seed: u64) -> Self {
        Self {
            layout: OracleLayout::new(width, height),
            wall: OracleWall::new(seed),
            pan: WallPan::resting(0.0),
            arrival: BlockArrival::finished(),
            feed: None,
            random: SplitMix::new(seed ^ 0xA90),
            seed,
            now: 0.0,
            since: 0.0,
            next_in: f64::INFINITY,
        }
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.layout = OracleLayout::new(width, height);
        let blocks = self.wall.blocks().to_vec();
        self.wall = OracleWall::new(self.seed);
        for block in blocks {
            self.settle(block);
        }
    }

    fn settle(&mut self, block: ChainBlock) {
        self.wall.push(block, &self.layout);
        self.wall.forget_hidden(&self.layout);
        self.pan = WallPan::resting(self.wall.ground_course() as f64);
    }

    pub fn advance(&mut self, step: f64) -> bool {
        self.now += step;
        self.since += step;
        self.arrival.advance(step);
        self.pan.advance(step);
        if self.pan.done() && self.wall.has_hidden() {
            self.wall.forget_hidden(&self.layout);
        }
        self.next_in -= step;
        if self.next_in > 0.0 {
            return false;
        }
        let Some(block) = self.feed.as_mut().map(BlockSimulator::next_block) else {
            return false;
        };
        self.next_in = (Self::SHORTEST_WAIT + (-self.random.unit().ln()) * Self::TARGET / Self::PACE).min(Self::TARGET / Self::PACE * 3.0);
        self.arrive(block)
    }

    pub fn arrive(&mut self, block: ChainBlock) -> bool {
        if self.wall.newest().is_some_and(|newest| newest.hash == block.hash) {
            return false;
        }
        self.wall.push(block, &self.layout);
        self.pan = self.pan.toward(self.wall.ground_course() as f64);
        self.arrival = BlockArrival::new();
        self.since = 0.0;
        true
    }

    pub fn fill(&mut self, blocks: &[ChainBlock]) {
        for block in blocks {
            self.settle(block.clone());
        }
    }

    #[must_use]
    pub const fn layout(&self) -> &OracleLayout {
        &self.layout
    }

    #[must_use]
    pub const fn wall(&self) -> &OracleWall {
        &self.wall
    }

    #[must_use]
    pub fn view(&self) -> f64 {
        self.pan.view()
    }

    #[must_use]
    pub const fn arrival(&self) -> &BlockArrival {
        &self.arrival
    }

    #[must_use]
    pub const fn now(&self) -> f64 {
        self.now
    }

    #[must_use]
    pub fn waited(&self) -> f64 {
        let pace = if self.feed.is_some() { Self::PACE } else { 1.0 };
        self.since * pace
    }

    #[must_use]
    pub fn simulated_feed(&self) -> bool {
        self.feed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::OracleScene;

    #[test]
    fn a_simulated_wall_starts_built_and_at_rest() {
        let scene = OracleScene::simulated(1200.0, 800.0, 3);

        assert!(scene.wall().blocks().len() >= 9);
        assert!(scene.arrival().done());
        assert!(scene.simulated_feed());
    }

    #[test]
    fn blocks_keep_arriving_and_each_one_starts_a_fresh_arrival() {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        let first = scene.wall().newest().map(|block| block.height);
        let mut arrivals = 0;
        for _ in 0..(120 * 60) {
            if scene.advance(1.0 / 60.0) {
                arrivals += 1;
                assert!(scene.arrival().age < 0.02 && scene.waited() < 1e-9);
            }
        }

        assert!((2..=10).contains(&arrivals), "{arrivals}");
        assert_eq!(scene.wall().newest().map(|block| block.height), first.map(|height| height + arrivals));
    }

    #[test]
    fn the_wait_is_shown_in_chain_seconds() {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        scene.advance(1.0);

        assert!((scene.waited() - OracleScene::PACE).abs() < 1e-9);
    }

    #[test]
    fn a_new_course_pans_the_wall_down_smoothly_and_drops_the_hidden_course_only_afterwards() {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        let start = scene.view();
        let mut views = vec![start];
        let mut hidden_while_moving = false;
        let mut finished = false;
        for _ in 0..(600 * 30) {
            scene.advance(1.0 / 30.0);
            let view = scene.view();
            if view > start + 1e-9 {
                views.push(view);
                hidden_while_moving |= view < start + 1.0 - 1e-9 && scene.wall().has_hidden();
                if (view - start - 1.0).abs() < 1e-9 && !scene.wall().has_hidden() {
                    finished = true;
                    break;
                }
            }
        }

        assert!(finished && hidden_while_moving);
        assert!(views.len() > 30, "{}", views.len());
        assert!(views.windows(2).all(|pair| pair[1] - pair[0] < 0.05 && pair[1] >= pair[0]));
    }

    #[test]
    fn resizing_keeps_the_same_blocks() {
        let mut scene = OracleScene::simulated(1200.0, 800.0, 3);
        let before: Vec<u32> = scene.wall().blocks().iter().map(|block| block.height).collect();
        scene.resize(1400.0, 800.0);
        let after: Vec<u32> = scene.wall().blocks().iter().map(|block| block.height).collect();

        assert_eq!(after.last(), before.last());
        assert!(after.len() >= 9);
    }
}
