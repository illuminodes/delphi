/// One part of a whole drawn as a stacked bar: what it is, how much, and its share of the bar.
/// `key` names the CSS colour class, `seg-{key}`.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitSegment {
    pub key: &'static str,
    pub label: &'static str,
    pub amount: String,
    pub share: f64,
    pub hint: &'static str,
}
