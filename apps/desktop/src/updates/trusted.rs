//! The update signature's trusted comment: the line the signature also covers, which binds a
//! payload to its feed entry. It must read exactly
//! `emberhut-desktop <platform> <arch> <version> build <build> sha256:<hex> notes:<hex>`, the last
//! being the SHA-256 of the notes' UTF-8 bytes as the feed's JSON decodes them (no trimming, no
//! normalising), so a feed can't relabel an old payload as a new release, pass a Mac payload off
//! as a Windows one, or change the notes the app shows. The format is
//! [`emberhut_api::releases::update_comment`]'s, which the release signer calls too.

use emberhut_api::releases::{Arch, Platform, Release, update_comment};

use super::choose::PRODUCT;

/// The trusted comment a payload of `release` for `platform` and `arch`, whose SHA-256 is
/// `sha256`, must be signed with.
#[must_use]
pub(crate) fn expected(
    release: &Release,
    platform: &Platform,
    arch: &Arch,
    sha256: &[u8; 32],
) -> String {
    update_comment(
        PRODUCT,
        platform,
        arch,
        &release.version,
        release.build,
        sha256,
        &release.notes,
    )
}

/// `bytes` as lowercase hex.
#[must_use]
pub(crate) fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sha2::{Digest as _, Sha256};

    use super::*;

    #[test]
    fn the_comment_names_the_entry_and_hashes_the_notes_as_they_are() {
        let release: Release = serde_json::from_value(json!({"version": "0.3.0", "build": 15,
            "date": "2026-12-08", "notes": "- Faster.\n"}))
        .unwrap();
        let payload = [0xab; 32];
        let comment = expected(&release, &Platform::Macos, &Arch::Arm64, &payload);
        let notes = hex(&Sha256::digest(b"- Faster.\n").into());
        assert_eq!(
            comment,
            format!(
                "emberhut-desktop macos arm64 0.3.0 build 15 sha256:{} notes:{notes}",
                "ab".repeat(32)
            )
        );
        let mut trimmed = release.clone();
        trimmed.notes = "- Faster.".into();
        assert_ne!(
            expected(&trimmed, &Platform::Macos, &Arch::Arm64, &payload),
            comment,
            "a newline is part of the notes"
        );
        let windows = expected(&release, &Platform::Windows, &Arch::X86_64, &payload);
        assert!(
            windows.starts_with("emberhut-desktop windows x86_64 0.3.0 build 15 sha256:"),
            "{windows}"
        );
    }

    #[test]
    fn hex_is_lowercase_and_two_digits_a_byte() {
        let mut bytes = [0; 32];
        bytes[0] = 0x0f;
        bytes[31] = 0xa0;
        let text = hex(&bytes);
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("0f00"));
        assert!(text.ends_with("00a0"));
    }
}
