pub mod chain_view;
pub mod error;
pub mod genesis;
pub mod merkle;
pub mod validation;

#[cfg(test)]
pub(crate) mod test_helpers {
    pub fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16)
                .unwrap_or_else(|_| panic!("invalid hex at offset {i}")))
            .collect()
    }

    pub fn hash_to_hex(hash: [u8; 32]) -> String {
        mantle::primitives::Hash256(hash).to_display_hex()
    }
}

#[cfg(test)]
mod tests_proptest;

#[cfg(test)]
mod tests_bip_vectors;
