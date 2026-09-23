//! Signing what a `json` destination receives.
//!
//! HMAC-SHA256 over `<timestamp>.<body>`, sent as
//! `X-Kasl-Signature: t=<timestamp>,sha256=<hex>`. The timestamp is inside
//! what is signed so a captured request cannot be replayed a week later with
//! a fresh header: the receiver checks the signature and refuses one whose
//! `t` is far from its own clock.
//!
//! Written here rather than taken as a crate: HMAC is two hashes and two
//! paddings over `sha2`, which the server already uses for agent tokens, and a
//! dependency for twenty lines is a dependency to keep current for twenty
//! lines. The RFC 4231 vectors below are what keep it honest.

use sha2::{Digest, Sha256};

/// SHA-256's block size, in bytes.
const BLOCK: usize = 64;

/// HMAC-SHA256 of `message` under `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let inner_pad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let outer_pad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();

    let inner = Sha256::new().chain_update(&inner_pad).chain_update(message).finalize();
    let outer = Sha256::new().chain_update(&outer_pad).chain_update(inner).finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&outer);
    out
}

/// The `X-Kasl-Signature` header's value for a body sent at `timestamp`
/// (seconds since the epoch).
pub fn signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    format!("t={timestamp},sha256={}", hex(&hmac_sha256(secret.as_bytes(), &signed)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn matches_the_rfc_4231_vectors() {
        // Case 1: a short key.
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Case 2: a key shorter than the block, as text.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Case 6: a key longer than the block, which is hashed first - the
        // branch a hand-written HMAC most often gets wrong.
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
        // Case 7: both long.
        let message = unhex(
            "5468697320697320612074657374207573696e672061206c6172676572207468616e20626c6f636b2d73697a65206b657920616e642061206c6172676572207468616e20626c6f636b2d73697a6520646174612e20546865206b6579206e6565647320746f20626520686173686564206265666f7265206265696e6720757365642062792074686520484d414320616c676f726974686d2e",
        );
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], &message)),
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"
        );
    }

    #[test]
    fn the_timestamp_is_part_of_what_is_signed() {
        let body = br#"{"event":"test"}"#;
        let now = signature("s", 1_700_000_000, body);
        let later = signature("s", 1_700_000_001, body);
        assert!(now.starts_with("t=1700000000,sha256="));
        assert_ne!(
            now.split_once(",sha256=").unwrap().1,
            later.split_once(",sha256=").unwrap().1,
            "a replay with a new timestamp must not carry a valid signature",
        );
    }
}
