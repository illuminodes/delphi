#![forbid(unsafe_code)]

pub mod amount_dump;
pub mod convergence;
pub mod granularity;
pub mod intraday_reference;
pub mod price_series;
pub mod reference_prices;
pub mod sample_rng;
pub mod score;
pub mod signal_density;
pub mod span_plan;
pub mod subsample_sweep;
pub mod volatility;
pub mod window_plan;

pub use amount_dump::AmountDump;
pub use convergence::{ConvergenceRow, ConvergenceTrial};
pub use granularity::{GranularityRow, GranularitySweep};
pub use intraday_reference::{IntradayError, IntradayReference};
pub use price_series::{PriceSeries, SeriesError};
pub use reference_prices::{ReferenceError, ReferencePrices};
pub use sample_rng::SampleRng;
pub use score::Scorecard;
pub use signal_density::SignalDensity;
pub use span_plan::{SpanPlan, SpanWindow};
pub use subsample_sweep::{DrawMode, PricedPool, SubsampleSweep};
pub use volatility::{VolatilityBand, VolatilityPoint, VolatilityStudy};
pub use window_plan::WindowPlan;
