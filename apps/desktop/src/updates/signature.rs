//! The update signature: a minisign signature (Ed25519, of the payload's `BLAKE2b` hash) by a key
//! this app trusts, whose trusted comment is exactly the one the feed entry calls for. The
//! payload is read from disk in chunks, never whole.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use minisign_verify::{PublicKey, Signature};

use super::keys::TrustedKey;

/// Why a payload's signature doesn't verify.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SignatureError {
    /// This app trusts no update key yet.
    #[error("this app trusts no update key yet")]
    NoTrustedKey,
    /// The signature isn't a minisign signature, or is the legacy kind that signs the bytes
    /// themselves rather than their hash.
    #[error("the signature isn't a minisign signature of the payload's hash")]
    Malformed,
    /// It was made with a key this app doesn't trust.
    #[error("the payload was signed with a key this app doesn't trust")]
    UnknownKey,
    /// It doesn't match the payload or its trusted comment.
    #[error("the signature doesn't match the payload")]
    Invalid,
    /// Its trusted comment names another release.
    #[error("the signature is for another release: {0}")]
    WrongRelease(String),
    /// The payload couldn't be read.
    #[error("couldn't read the payload")]
    Read(#[source] io::Error),
}

/// Checks that `signature` (a minisign signature file, as text) signs the file at `payload`
/// with one of `keys`, with `comment` as its trusted comment.
///
/// # Errors
///
/// What [`SignatureError`] lists.
pub(crate) fn verify(
    payload: &Path,
    signature: &str,
    comment: &str,
    keys: &[TrustedKey],
) -> Result<(), SignatureError> {
    if keys.is_empty() {
        return Err(SignatureError::NoTrustedKey);
    }
    let signature = Signature::decode(signature).map_err(|_| SignatureError::Malformed)?;
    for key in keys {
        let public = match PublicKey::from_base64(key.public) {
            Ok(public) => public,
            Err(error) => {
                log::error!("the trusted update key {} doesn't read: {error}", key.id);
                continue;
            }
        };
        let mut verifier = match public.verify_stream(&signature) {
            Ok(verifier) => verifier,
            Err(minisign_verify::Error::UnexpectedKeyId) => continue,
            Err(_) => return Err(SignatureError::Malformed),
        };
        let mut file = File::open(payload).map_err(SignatureError::Read)?;
        let mut buffer = vec![0; 64 << 10];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => verifier.update(&buffer[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(SignatureError::Read(error)),
            }
        }
        verifier.finalize().map_err(|_| SignatureError::Invalid)?;
        // The comment is signed too (finalize checked it): now it must be this release's.
        return if signature.trusted_comment() == comment {
            log::info!("the update is signed by trusted key {}", key.id);
            Ok(())
        } else {
            Err(SignatureError::WrongRelease(
                signature.trusted_comment().to_owned(),
            ))
        };
    }
    Err(SignatureError::UnknownKey)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    /// A key pair made for a test, and its public half as a trusted key.
    pub(crate) struct TestKey {
        pair: minisign::KeyPair,
        /// The public half, as the app's list holds it.
        pub(crate) trusted: TrustedKey,
    }

    impl TestKey {
        pub(crate) fn new() -> Self {
            let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            let public: &'static str = Box::leak(pair.pk.to_base64().into_boxed_str());
            Self {
                trusted: TrustedKey {
                    id: "0123456789ABCDEF",
                    public,
                },
                pair,
            }
        }

        /// `bytes` signed with `comment` as the trusted comment, as a signature file's text.
        pub(crate) fn sign(&self, bytes: &[u8], comment: &str) -> String {
            minisign::sign(
                Some(&self.pair.pk),
                &self.pair.sk,
                Cursor::new(bytes),
                Some(comment),
                Some("signature from a test key"),
            )
            .unwrap()
            .to_string()
        }
    }

    fn payload(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Emberhut.dmg");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn a_payload_signed_by_a_trusted_key_with_its_comment_verifies() {
        let key = TestKey::new();
        let (_dir, path) = payload(b"a disk image");
        let signature = key.sign(
            b"a disk image",
            "emberhut-desktop macos arm64 0.3.0 build 15",
        );
        verify(
            &path,
            &signature,
            "emberhut-desktop macos arm64 0.3.0 build 15",
            &[key.trusted],
        )
        .unwrap();
    }

    #[test]
    fn no_trusted_key_fails_closed() {
        let key = TestKey::new();
        let (_dir, path) = payload(b"a disk image");
        let signature = key.sign(b"a disk image", "c");
        assert!(matches!(
            verify(&path, &signature, "c", &[]),
            Err(SignatureError::NoTrustedKey)
        ));
        assert!(matches!(
            verify(&path, &signature, "c", super::super::keys::TRUSTED),
            Err(SignatureError::NoTrustedKey)
        ));
    }

    #[test]
    fn another_key_other_bytes_or_another_comment_fail() {
        let (key, stranger) = (TestKey::new(), TestKey::new());
        let (_dir, path) = payload(b"a disk image");
        let by_stranger = stranger.sign(b"a disk image", "c");
        assert!(matches!(
            verify(&path, &by_stranger, "c", &[key.trusted]),
            Err(SignatureError::UnknownKey)
        ));
        let of_other_bytes = key.sign(b"another disk image", "c");
        assert!(matches!(
            verify(&path, &of_other_bytes, "c", &[key.trusted]),
            Err(SignatureError::Invalid)
        ));
        let old = key.sign(
            b"a disk image",
            "emberhut-desktop macos arm64 0.2.0 build 12",
        );
        assert!(matches!(
            verify(&path, &old, "emberhut-desktop macos arm64 0.3.0 build 15", &[key.trusted]),
            Err(SignatureError::WrongRelease(comment)) if comment.ends_with("build 12")
        ));
        assert!(matches!(
            verify(&path, "not a signature", "c", &[key.trusted]),
            Err(SignatureError::Malformed)
        ));
    }

    #[test]
    fn a_trusted_comment_changed_after_signing_fails() {
        let key = TestKey::new();
        let (_dir, path) = payload(b"a disk image");
        let signature = key.sign(b"a disk image", "build 12");
        let forged = signature.replace("trusted comment: build 12", "trusted comment: build 15");
        assert_ne!(forged, signature);
        assert!(matches!(
            verify(&path, &forged, "build 15", &[key.trusted]),
            Err(SignatureError::Invalid)
        ));
    }

    #[test]
    fn a_legacy_signature_of_the_bytes_themselves_is_refused() {
        // minisign's own example: a legacy (non-prehashed) signature of "test".
        let trusted = TrustedKey {
            id: "E7620F1842B4E81F",
            public: "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
        };
        let signature = "untrusted comment: signature from minisign secret key\n\
            RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
            trusted comment: timestamp:1555779966\tfile:test\n\
            QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let (_dir, path) = payload(b"test");
        assert!(matches!(
            verify(
                &path,
                signature,
                "timestamp:1555779966\tfile:test",
                &[trusted]
            ),
            Err(SignatureError::Malformed)
        ));
    }
}
