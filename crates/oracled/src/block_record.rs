use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::hash::Hash256;
use mantle::tx::Transaction;
use delphi_oracle::{AdmittedSample, OutputFilter, TransactionTally};

/// The outputs one block contributes to the price, after filtering.
///
/// Filtering sees only this block, so a spend of an output created in an
/// earlier block is not detected. That keeps a record independent of every
/// other block and small enough to hold a week of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
    height: BlockHeight,
    time: BlockTime,
    hash: Hash256,
    amounts: Vec<Satoshi>,
}

impl BlockRecord {
    const LEGACY_HEADER_BYTES: usize = 12;
    const HEADER_BYTES: usize = 44;
    const AMOUNT_BYTES: usize = 8;

    #[must_use]
    pub fn new(height: BlockHeight, time: BlockTime, hash: Hash256, amounts: Vec<Satoshi>) -> Self {
        Self {
            height,
            time,
            hash,
            amounts,
        }
    }

    #[must_use]
    pub fn from_transactions<'a>(
        height: BlockHeight,
        time: BlockTime,
        hash: Hash256,
        transactions: impl IntoIterator<Item = &'a Transaction>,
    ) -> Self {
        Self::screened(height, time, hash, transactions).0
    }

    /// [`Self::from_transactions`] together with the verdict on every transaction.
    #[must_use]
    pub fn screened<'a>(
        height: BlockHeight,
        time: BlockTime,
        hash: Hash256,
        transactions: impl IntoIterator<Item = &'a Transaction>,
    ) -> (Self, TransactionTally) {
        let mut filter = OutputFilter::new();
        let mut tally = TransactionTally::default();
        let mut amounts = Vec::new();
        for transaction in transactions {
            let (verdict, admitted) = filter.screen(transaction);
            tally.record(verdict, transaction.outputs.len(), admitted.len());
            amounts.extend(admitted);
        }
        (Self::new(height, time, hash, amounts), tally)
    }

    #[must_use]
    pub const fn height(&self) -> BlockHeight {
        self.height
    }

    #[must_use]
    pub const fn time(&self) -> BlockTime {
        self.time
    }

    #[must_use]
    pub const fn hash(&self) -> Hash256 {
        self.hash
    }

    /// Whether the record predates stored hashes and still needs one.
    #[must_use]
    pub fn lacks_hash(&self) -> bool {
        self.hash == Hash256::ZERO
    }

    pub const fn set_hash(&mut self, hash: Hash256) {
        self.hash = hash;
    }

    #[must_use]
    pub fn amounts(&self) -> &[Satoshi] {
        &self.amounts
    }

    pub fn push_into(&self, sample: &mut AdmittedSample) {
        for amount in &self.amounts {
            sample.push(*amount, self.height, self.time);
        }
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.height.to_u32().to_le_bytes());
        out.extend_from_slice(&self.time.to_u32().to_le_bytes());
        out.extend_from_slice(self.hash.as_bytes());
        let count = u32::try_from(self.amounts.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&count.to_le_bytes());
        for amount in self.amounts.iter().take(count as usize) {
            out.extend_from_slice(&amount.to_u64().to_le_bytes());
        }
    }

    /// Decode one record from the front of `bytes`, returning it with the
    /// number of bytes consumed, or `None` when the bytes end mid-record.
    #[must_use]
    pub fn decode_prefix(bytes: &[u8]) -> Option<(Self, usize)> {
        let header = bytes.get(..Self::HEADER_BYTES)?;
        let hash = Hash256::from_bytes(header[8..40].try_into().ok()?);
        Self::decode_with(bytes, Self::HEADER_BYTES, hash)
    }

    /// Decode one record written before hashes were stored. The hash comes
    /// back as [`Hash256::ZERO`] until filled in from a header.
    #[must_use]
    pub fn decode_legacy_prefix(bytes: &[u8]) -> Option<(Self, usize)> {
        Self::decode_with(bytes, Self::LEGACY_HEADER_BYTES, Hash256::ZERO)
    }

    fn decode_with(bytes: &[u8], header_bytes: usize, hash: Hash256) -> Option<(Self, usize)> {
        let header = bytes.get(..header_bytes)?;
        let height = BlockHeight::new(Self::word(&header[0..4])?);
        let time = BlockTime::new(Self::word(&header[4..8])?);
        let count = usize::try_from(Self::word(&header[header_bytes - 4..header_bytes])?).ok()?;
        let total = header_bytes.checked_add(count.checked_mul(Self::AMOUNT_BYTES)?)?;
        let body = bytes.get(header_bytes..total)?;
        let amounts = body
            .chunks_exact(Self::AMOUNT_BYTES)
            .map(|chunk| chunk.try_into().ok().map(u64::from_le_bytes))
            .map(|value| value.map(Satoshi::new))
            .collect::<Option<Vec<_>>>()?;
        Some((Self::new(height, time, hash, amounts), total))
    }

    fn word(bytes: &[u8]) -> Option<u32> {
        bytes.try_into().ok().map(u32::from_le_bytes)
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use delphi_oracle::AdmittedSample;

    use super::BlockRecord;

    struct RecordFixture;

    impl RecordFixture {
        fn record(height: u32, amounts: &[u64]) -> BlockRecord {
            BlockRecord::new(
                BlockHeight::new(height),
                BlockTime::new(1_700_000_000 + height),
                Hash256::hash(&height.to_le_bytes()),
                amounts.iter().copied().map(Satoshi::new).collect(),
            )
        }

        fn legacy_bytes(height: u32, time: u32, amounts: &[u64]) -> Vec<u8> {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&height.to_le_bytes());
            bytes.extend_from_slice(&time.to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(amounts.len()).expect("small").to_le_bytes());
            for amount in amounts {
                bytes.extend_from_slice(&amount.to_le_bytes());
            }
            bytes
        }
    }

    #[test]
    fn a_legacy_record_decodes_without_a_hash() {
        let bytes = RecordFixture::legacy_bytes(12, 34, &[5_000, 6_000]);

        let (record, used) = BlockRecord::decode_legacy_prefix(&bytes).expect("legacy record");

        assert_eq!(used, bytes.len());
        assert_eq!(record.height(), BlockHeight::new(12));
        assert_eq!(record.time(), BlockTime::new(34));
        assert_eq!(record.amounts().len(), 2);
        assert!(record.lacks_hash());
    }

    #[test]
    fn the_hash_survives_a_round_trip() {
        let record = RecordFixture::record(3, &[1]);
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes);

        let (decoded, _) = BlockRecord::decode_prefix(&bytes).expect("record");

        assert_eq!(decoded.hash(), Hash256::hash(&3_u32.to_le_bytes()));
        assert!(!decoded.lacks_hash());
    }

    #[test]
    fn a_record_survives_an_encode_decode_round_trip() {
        let record = RecordFixture::record(900_000, &[50_000, 123_456, 9_999_999]);
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes);

        let (decoded, used) = BlockRecord::decode_prefix(&bytes).expect("complete record");

        assert_eq!(decoded, record);
        assert_eq!(used, bytes.len());
    }

    #[test]
    fn an_empty_record_round_trips() {
        let record = RecordFixture::record(5, &[]);
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes);

        let (decoded, used) = BlockRecord::decode_prefix(&bytes).expect("complete record");

        assert_eq!(decoded, record);
        assert_eq!(used, 44);
    }

    #[test]
    fn a_truncated_record_is_not_decoded() {
        let mut bytes = Vec::new();
        RecordFixture::record(7, &[1_000, 2_000]).encode_into(&mut bytes);

        for cut in 0..bytes.len() {
            assert!(BlockRecord::decode_prefix(&bytes[..cut]).is_none(), "cut {cut}");
        }
    }

    #[test]
    fn a_huge_declared_count_does_not_overflow() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());

        assert!(BlockRecord::decode_prefix(&bytes).is_none());
        assert!(BlockRecord::decode_legacy_prefix(&RecordFixture::legacy_bytes(1, 2, &[])[..8]).is_none());
    }

    #[test]
    fn consecutive_records_decode_in_order() {
        let mut bytes = Vec::new();
        RecordFixture::record(1, &[1_500]).encode_into(&mut bytes);
        RecordFixture::record(2, &[2_500, 3_500]).encode_into(&mut bytes);

        let (first, used) = BlockRecord::decode_prefix(&bytes).expect("first");
        let (second, _) = BlockRecord::decode_prefix(&bytes[used..]).expect("second");

        assert_eq!(first.height(), BlockHeight::new(1));
        assert_eq!(second.amounts().len(), 2);
    }

    #[test]
    fn pushing_into_a_sample_carries_height_and_time() {
        let record = RecordFixture::record(42, &[10_000, 20_000]);
        let mut sample = AdmittedSample::new();

        record.push_into(&mut sample);

        assert_eq!(sample.len(), 2);
        assert_eq!(sample.outputs()[0].height, BlockHeight::new(42));
        assert_eq!(sample.outputs()[1].time, record.time());
    }
}
