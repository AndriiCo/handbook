//! 64-bit SimHash over sidecar text: the boilerplate-robust change
//! gate between fetch and ingest.
//!
//! The standard crawler recipe: Hamming distance ≤ 3 means
//! near-duplicate — template churn (nav, timestamps, ads) that must
//! not mint a new capture. Hashing is hand-rolled FNV-1a (no crate
//! earns its place for this) and deterministic across processes
//! (unlike `std`'s randomly-seeded `SipHash`, which would make stored
//! fingerprints uncomparable after a restart).

/// Hamming distance at or below which two sidecars are near-duplicates.
pub const NEAR_DUP_DISTANCE: u32 = 3;

/// FNV-1a 64: deterministic token hash (fixed offsets, no seed).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// SimHash fingerprint of `text`. Tokens are lowercase ASCII-alphanumeric
/// runs; anything else splits. Empty text fingerprints to 0.
pub fn simhash(text: &str) -> u64 {
    let mut acc = [0i32; 64];
    let mut token = Vec::new();
    let flush = |token: &mut Vec<u8>, acc: &mut [i32; 64]| {
        if token.is_empty() {
            return;
        }
        let hash = fnv1a64(token);
        for (i, slot) in acc.iter_mut().enumerate() {
            *slot += if (hash >> i) & 1 == 1 { 1 } else { -1 };
        }
        token.clear();
    };
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() {
            token.push(byte.to_ascii_lowercase());
        } else {
            flush(&mut token, &mut acc);
        }
    }
    flush(&mut token, &mut acc);
    let mut out = 0u64;
    for (i, slot) in acc.iter().enumerate() {
        if *slot > 0 {
            out |= 1u64 << i;
        }
    }
    out
}

/// Bit differences between two fingerprints.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// True when `candidate` is a near-duplicate of the stored fingerprint
/// (and a stored fingerprint exists — 0 means "never fingerprinted",
/// which must not match everything).
pub fn is_near_dup(stored: u64, candidate: u64) -> bool {
    stored != 0 && hamming(stored, candidate) <= NEAR_DUP_DISTANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_share_a_fingerprint() {
        let text = "PO Box 40220\nCounter fee $8\nMon-Fri 9-5\n";
        assert_eq!(simhash(text), simhash(text));
    }

    #[test]
    fn one_word_change_is_near_but_rewrite_is_far() {
        // Page-shaped (~200 tokens): the ≤3 threshold is calibrated for
        // real sidecars, not sentences.
        let para = "office hours monday tuesday wednesday thursday friday \
            counter fee eight dollars mailing address post office box four \
            zero two two zero olympia washington secretary of state \
            apostille authentication notary commission federal express ";
        let base = para.repeat(4);
        let tweaked = base.replacen("eight", "nine", 1);
        let rewritten = "completely unrelated prose about maritime shipping \
            routes through the panama canal and suez canal tolls";
        let near = hamming(simhash(&base), simhash(&tweaked));
        let far = hamming(simhash(&base), simhash(rewritten));
        assert!(near <= NEAR_DUP_DISTANCE, "near: {near}");
        assert!(far > NEAR_DUP_DISTANCE, "far: {far}");
        assert!(is_near_dup(simhash(&base), simhash(&tweaked)));
        assert!(!is_near_dup(simhash(&base), simhash(rewritten)));
    }

    #[test]
    fn empty_text_fingerprints_zero_and_never_matches() {
        assert_eq!(simhash(""), 0);
        assert_eq!(simhash("   \n\t  "), 0);
        assert!(!is_near_dup(0, simhash("anything")));
        assert!(!is_near_dup(simhash("anything"), 0));
    }

    #[test]
    fn hamming_counts_bits() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(0, 1), 1);
        assert_eq!(hamming(0, u64::MAX), 64);
        assert_eq!(hamming(0b1010, 0b1001), 2);
    }
}
