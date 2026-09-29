use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;
use mantle_oracle::{MempoolCollector, PriceBand};

use crate::{BootstrapRng, MempoolEstimator, PriceSource, PriceUpdate};

/// Unconfirmed transactions seen since the last confirmed block.
///
/// A new block confirms most of what the pool held, so the collector is
/// discarded and started again whenever the tip moves. The window is
/// therefore bounded by block time rather than by a transaction count.
pub struct MempoolWindow {
    name: String,
    collector: MempoolCollector,
    estimator: MempoolEstimator,
    rng: BootstrapRng,
}

impl MempoolWindow {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        height: BlockHeight,
        estimator: MempoolEstimator,
        seed: u64,
    ) -> Self {
        Self {
            name: name.into(),
            collector: MempoolCollector::new(height),
            estimator,
            rng: BootstrapRng::seeded(seed),
        }
    }

    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime) -> usize {
        self.collector.absorb(transaction, at)
    }

    /// Discard the pool and start counting from a new tip.
    pub fn reset(&mut self, height: BlockHeight) {
        self.collector = MempoolCollector::new(height);
    }

    #[must_use]
    pub fn outputs(&self) -> usize {
        self.collector.sample().len()
    }

    #[must_use]
    pub fn admitted_transactions(&self) -> u64 {
        self.collector.admitted_transactions()
    }

    /// The window's current price inside `band`, or `None` while the sample
    /// is thin or no price falls inside the band.
    pub fn update(&mut self, band: PriceBand) -> Option<PriceUpdate> {
        let estimate = self
            .estimator
            .estimate(self.collector.sample(), band, &mut self.rng)?;
        Some(PriceUpdate::new(
            self.name.clone(),
            estimate.price,
            estimate.outputs,
            PriceSource::Mempool {
                transactions: usize::try_from(self.collector.admitted_transactions())
                    .unwrap_or(usize::MAX),
                band_low: estimate.band_low,
                band_high: estimate.band_high,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime, Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};
    use mantle_oracle::PriceBand;

    use super::MempoolWindow;
    use crate::{BootstrapRng, MempoolEstimator, PriceSource};

    struct PoolFixture {
        counter: u32,
        rng: BootstrapRng,
    }

    impl PoolFixture {
        const ROUND_USD: [f64; 9] = [5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1_000.0];

        fn new() -> Self {
            Self {
                counter: 0,
                rng: BootstrapRng::seeded(11),
            }
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        fn output(&mut self, price: f64) -> Satoshi {
            let usd = if self.rng.below(3) == 0 {
                let fraction =
                    f64::from(u32::try_from(self.rng.next_u64() % 10_000).unwrap_or(0)) / 10_000.0;
                5.0 * 400.0_f64.powf(fraction)
            } else {
                Self::ROUND_USD[self.rng.below(Self::ROUND_USD.len())]
            };
            Satoshi::new(((usd / price * 100_000_000.0).round() as u64).max(2_000))
        }

        fn transaction(&mut self, price: f64) -> Transaction {
            self.counter += 1;
            let mut txid = [0_u8; 32];
            txid[..4].copy_from_slice(&self.counter.to_le_bytes());
            let outputs = (0..2)
                .map(|_| TxOut {
                    value: self.output(price),
                    script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                })
                .collect();
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes(txid),
                        vout: 0,
                    },
                    script_sig: ScriptSig::from_slice(&[]),
                    sequence: Sequence::new(u32::MAX),
                    witness: Witness::empty(),
                }],
                outputs,
                locktime: Locktime::new(0),
            }
        }

        fn fill(&mut self, window: &mut MempoolWindow, transactions: usize, price: f64) {
            for _ in 0..transactions {
                let transaction = self.transaction(price);
                window.absorb(&transaction, BlockTime::new(1));
            }
        }
    }

    fn window() -> MempoolWindow {
        MempoolWindow::new("mempool", BlockHeight::new(1), MempoolEstimator::new(200, 20), 5)
    }

    fn band() -> PriceBand {
        PriceBand::around(80_000.0, 0.15).expect("band")
    }

    #[test]
    fn an_empty_window_has_no_update() {
        assert!(window().update(band()).is_none());
    }

    #[test]
    fn a_thin_window_has_no_update() {
        let mut fixture = PoolFixture::new();
        let mut window = window();
        fixture.fill(&mut window, 30, 80_000.0);

        assert!(window.outputs() < 200);
        assert!(window.update(band()).is_none());
    }

    #[test]
    fn a_full_window_prices_near_the_planted_price() {
        let mut fixture = PoolFixture::new();
        let mut window = window();
        fixture.fill(&mut window, 300, 80_000.0);
        let update = window.update(band()).expect("update");

        assert_eq!(update.window(), "mempool");
        assert!((update.price() - 80_000.0).abs() / 80_000.0 < 0.02, "{}", update.price());
        assert_eq!(update.samples(), window.outputs());
        let PriceSource::Mempool {
            transactions,
            band_low,
            band_high,
        } = *update.source()
        else {
            panic!("mempool source expected");
        };
        assert_eq!(transactions as u64, window.admitted_transactions());
        assert!(band_low <= update.price() + 1.0 && update.price() <= band_high + 1.0);
        assert!(update.is_publishable());
    }

    #[test]
    fn a_new_block_empties_the_window() {
        let mut fixture = PoolFixture::new();
        let mut window = window();
        fixture.fill(&mut window, 300, 80_000.0);
        window.reset(BlockHeight::new(2));

        assert_eq!(window.outputs(), 0);
        assert_eq!(window.admitted_transactions(), 0);
        assert!(window.update(band()).is_none());
    }

    #[test]
    fn the_window_refills_after_a_reset() {
        let mut fixture = PoolFixture::new();
        let mut window = window();
        fixture.fill(&mut window, 300, 80_000.0);
        window.reset(BlockHeight::new(2));
        fixture.fill(&mut window, 300, 80_000.0);

        assert!(window.update(band()).is_some());
    }

    #[test]
    fn a_band_around_a_far_price_yields_nothing_near_the_true_price() {
        let mut fixture = PoolFixture::new();
        let mut window = window();
        fixture.fill(&mut window, 300, 80_000.0);
        let far = PriceBand::around(160_000.0, 0.05).expect("band");

        assert!(window.update(far).is_none_or(|update| far.contains(update.price())));
    }
}
