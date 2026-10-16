use std::sync::atomic::{AtomicU32, Ordering};

pub struct ElementId;

impl ElementId {
    #[must_use]
    pub fn next(prefix: &str) -> String {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::ElementId;

    #[test]
    fn every_id_is_new_and_keeps_its_prefix() {
        let first = ElementId::next("hatch");
        let second = ElementId::next("hatch");

        assert_ne!(first, second);
        assert!(first.starts_with("hatch-") && second.starts_with("hatch-"));
    }
}
