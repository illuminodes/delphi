use std::thread::{self, JoinHandle};

use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::spsc::{Consumer, Producer, RingBuffer};

use crate::{BootstrapRng, EstimateJob, Event, EventSender, MempoolEstimator};

/// Runs mempool bootstraps off the hub thread.
///
/// The hub submits at most [`Self::QUEUE`] jobs ahead; a submit while the
/// worker is still busy is skipped, since the next tick brings a fresher
/// sample. Results come back to the hub as [`Event::Estimate`].
pub struct EstimatorWorker {
    jobs: Consumer<EstimateJob>,
    estimator: MempoolEstimator,
    rng: BootstrapRng,
    events: EventSender,
}

/// The hub's end: submit jobs, and stop the worker when done.
pub struct EstimatorHandle {
    jobs: Option<Producer<EstimateJob>>,
    thread: Option<JoinHandle<()>>,
}

impl EstimatorWorker {
    pub const QUEUE: usize = 1;

    /// # Errors
    /// Returns a message when the thread cannot be spawned.
    pub fn spawn(estimator: MempoolEstimator, seed: u64, events: EventSender) -> Result<EstimatorHandle, String> {
        let (producer, consumer) = RingBuffer::new(Capacity::exact(Self::QUEUE)).split();
        let worker = Self {
            jobs: consumer,
            estimator,
            rng: BootstrapRng::seeded(seed),
            events,
        };
        let thread = thread::Builder::new()
            .name("estimator".into())
            .spawn(move || worker.run())
            .map_err(|error| format!("spawn estimator: {error}"))?;
        Ok(EstimatorHandle {
            jobs: Some(producer),
            thread: Some(thread),
        })
    }

    fn run(mut self) {
        while let Some(job) = self.jobs.pop_block() {
            let result = job.run(&self.estimator, &mut self.rng);
            if !self.events.send(Event::Estimate(Box::new(result))) {
                return;
            }
        }
    }
}

impl EstimatorHandle {
    /// Queue `job`; `false` when the worker is still busy with an earlier one.
    pub fn submit(&self, job: EstimateJob) -> bool {
        self.jobs.as_ref().is_some_and(|jobs| jobs.push(job).is_ok())
    }

    /// Close the queue and wait for the worker to finish its job.
    pub fn stop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for EstimatorHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;
    use delphi_oracle::PriceBand;

    use super::EstimatorWorker;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{EstimateJob, Event, EventBus, MempoolEstimator};

    #[test]
    fn a_submitted_job_comes_back_as_an_estimate_event() {
        let (sender, mut receiver) = EventBus::open(8);
        let mut handle = EstimatorWorker::spawn(MempoolEstimator::new(200, 10), 1, sender).expect("spawn");
        let job = EstimateJob {
            name: "mempool".to_string(),
            sample: SyntheticMarket::new(80_000.0, 3).sample(600),
            admitted_transactions: 300,
            band: PriceBand::around(80_000.0, 0.15).expect("band"),
            tip: Some(BlockHeight::new(9)),
        };

        assert!(handle.submit(job));
        let Some(Event::Estimate(result)) = receiver.recv() else {
            panic!("estimate expected");
        };
        handle.stop();

        assert_eq!(result.tip, Some(BlockHeight::new(9)));
        assert!(result.update.is_some());
        assert!(receiver.recv().is_none());
    }
}
