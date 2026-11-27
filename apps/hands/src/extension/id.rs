//! A Chrome extension's id from the public key in its manifest's `key`.

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

/// The id Chrome gives Emberhut's browser extension when it is loaded unpacked: the first 128 bits
/// of the SHA-256 of the public key in its manifest's `key`, written with the letters `a` to
/// `p`. It is the id emberhut-api names for the extension (`chrome::extension::ID`), and must stay
/// equal to it.
pub const EXTENSION_ID: &str = "omnhmmnnbibfdohdhcbcomdhbgpehfnj";

/// The id Chrome gives an extension whose manifest `key` is `key` (base64 of the public key's
/// DER), or `None` when it isn't base64.
#[must_use]
pub fn extension_id(key: &str) -> Option<String> {
    let compact: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .ok()
        .filter(|der| !der.is_empty())?;
    let digest = Sha256::digest(der);
    Some(
        digest
            .iter()
            .take(16)
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .map(|nibble| char::from(b'a' + nibble))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_the_key_s_hash_in_letters_a_to_p() {
        // SHA-256("abc") starts ba 78 16 bf 8f 01 cf ea 41 41 40 de 5d ae 22 23.
        let id = extension_id("YWJj").unwrap();
        assert_eq!(id, "lkhibglpipabmpokebebeanofnkocccd");
        assert_eq!(id.len(), 32);
        assert_eq!(extension_id(" YW\nJj "), Some(id));
    }

    #[test]
    fn a_key_that_isn_t_base64_has_no_id() {
        assert_eq!(extension_id("not base64!"), None);
        assert_eq!(extension_id(""), None);
    }

    #[test]
    fn the_fixed_id_reads_as_an_extension_id() {
        assert_eq!(EXTENSION_ID.len(), 32);
        assert!(EXTENSION_ID.bytes().all(|b| (b'a'..=b'p').contains(&b)));
    }

    #[test]
    fn the_fixed_id_is_the_server_s_and_the_extension_s_key_gives_it() {
        assert_eq!(EXTENSION_ID, emberhut_api::chrome::extension::ID);
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../extension/manifest.json")).unwrap();
        let key = manifest["key"].as_str().unwrap();
        assert_eq!(extension_id(key).as_deref(), Some(EXTENSION_ID));
    }
}
