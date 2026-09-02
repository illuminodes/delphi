#![forbid(unsafe_code)]

pub mod granularity;
pub mod intraday_reference;
pub mod price_series;
pub mod reference_prices;
pub mod score;
pub mod span_plan;
pub mod window_plan;

pub use granularity::{GranularityRow, GranularitySweep};
pub use intraday_reference::{IntradayError, IntradayReference};
pub use price_series::{PriceSeries, SeriesError};
pub use reference_prices::{ReferenceError, ReferencePrices};
pub use score::Scorecard;
pub use span_plan::{SpanPlan, SpanWindow};
pub use window_plan::WindowPlan;
