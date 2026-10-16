use crate::{DisplayFormat, PriceNote, PriceOrigin};

/// One short labelled value shown under a price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceFact {
    pub label: &'static str,
    pub value: String,
    pub hint: Option<String>,
}

impl PriceFact {
    const HASH_TAIL: usize = 8;

    const fn new(label: &'static str, value: String) -> Self {
        Self { label, value, hint: None }
    }

    #[must_use]
    pub fn list(note: &PriceNote, spread: Option<f64>) -> Vec<Self> {
        let mut facts = Vec::new();
        match note.origin() {
            PriceOrigin::Chain(detail) => {
                facts.push(Self::new(
                    "blocks",
                    format!(
                        "{}–{}",
                        DisplayFormat::count(detail.start_height as usize),
                        DisplayFormat::count(detail.stop_height as usize)
                    ),
                ));
                if let Some(deviation) = detail.deviation {
                    facts.push(Self::new("dev", format!("{deviation:.3}")));
                }
            }
            PriceOrigin::Mempool(detail) => {
                facts.push(Self::new("tx", DisplayFormat::count(detail.transactions)));
                if let Some((low, high)) = note.band() {
                    facts.push(Self::new(
                        "band",
                        format!("{}–{}", DisplayFormat::usd_whole(low), DisplayFormat::usd_whole(high)),
                    ));
                }
            }
        }
        facts.push(Self::new("samples", DisplayFormat::count(note.samples())));
        if let Some(spread) = spread {
            facts.push(Self::new("vs hourly", DisplayFormat::percent(spread)));
        }
        if let PriceOrigin::Chain(detail) = note.origin() {
            if let Some(hash) = &detail.stop_hash {
                let start = hash
                    .char_indices()
                    .rev()
                    .nth(Self::HASH_TAIL - 1)
                    .map_or(0, |(index, _)| index);
                facts.push(Self {
                    label: "hash",
                    value: format!("…{}", &hash[start..]),
                    hint: Some(hash.clone()),
                });
            }
        }
        facts
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::PriceFact;
    use crate::{NoteKind, PriceNote};

    struct FactFixture;

    impl FactFixture {
        fn note(kind: u32, window: &str, content: &str) -> PriceNote {
            let note = NostrNoteBuilder::new()
                .kind(kind)
                .content(content)
                .tag("w", window)
                .build();
            PriceNote::from_note(&note).expect("price")
        }

        fn chain() -> PriceNote {
            Self::note(
                NoteKind::AUDIT,
                "chain-1d",
                r#"{"stop_hash":"0000aaaa11112222","price":83551.5,"samples":7336,"deviation":0.158,"start_height":900000,"stop_height":900143,"start_time":1,"stop_time":2}"#,
            )
        }

        fn mempool() -> PriceNote {
            Self::note(
                NoteKind::LIVE,
                "mempool",
                r#"{"source":"mempool","price":83520.25,"samples":783,"transactions":401,"band_low":83400.0,"band_high":83640.5}"#,
            )
        }

        fn labels(facts: &[PriceFact]) -> Vec<&'static str> {
            facts.iter().map(|fact| fact.label).collect()
        }
    }

    #[test]
    fn a_chain_price_lists_its_blocks_deviation_and_hash() {
        let facts = PriceFact::list(&FactFixture::chain(), Some(0.25));

        assert_eq!(
            FactFixture::labels(&facts),
            vec!["blocks", "dev", "samples", "vs hourly", "hash"]
        );
        assert_eq!(facts[0].value, "900,000–900,143");
        assert_eq!(facts[4].value, "…11112222");
        assert_eq!(facts[4].hint.as_deref(), Some("0000aaaa11112222"));
    }

    #[test]
    fn a_mempool_price_lists_its_transactions_and_band() {
        let facts = PriceFact::list(&FactFixture::mempool(), None);

        assert_eq!(FactFixture::labels(&facts), vec!["tx", "band", "samples"]);
        assert_eq!(facts[1].value, "$83,400–$83,641");
    }
}
