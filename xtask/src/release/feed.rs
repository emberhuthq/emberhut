//! The updater's manifests for a GitHub release ("the feed"), `SHA256SUMS` and their
//! signatures, made with the update key, and the check that reads them back as the app does.
//!
//! Each desktop installer gets one feed, `emberhut-desktop-<platform>.json`: an
//! [`emberhut_api::releases::ReleaseFeed`] with this one release, whose assets point at the
//! installer by its absolute GitHub download URL. Each asset's signature is a minisign signature
//! of the installer with [`emberhut_api::releases::update_comment`] as its trusted comment, the
//! line the app checks. The universal macOS disk image is listed twice, for `arm64` and
//! `x86_64`, each with its own signature, since the comment names the arch.

use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail, ensure};
use emberhut_api::releases::{
    Arch, Asset, Build, FORMAT, Platform, Release, ReleaseFeed, update_comment,
};
use emberhut_api::time::{Day, Timestamp};
use minisign::{PublicKey, SecretKey, SecretKeyBox};
use sha2::{Digest as _, Sha256};

use crate::release::tag::Tag;

/// The repository whose releases carry the assets.
pub(crate) const REPOSITORY: &str = "emberhuthq/emberhut";
/// The product the desktop's feeds publish.
pub(crate) const PRODUCT: &str = "emberhut-desktop";
/// The checksum list.
pub(crate) const SUMS: &str = "SHA256SUMS";
/// The checksum list's signature.
pub(crate) const SUMS_SIGNATURE: &str = "SHA256SUMS.minisig";

/// A desktop installer the feed lists: its platform, file name and the processors it runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installer {
    /// The feed's platform.
    pub(crate) platform: Platform,
    /// The installer's file name for `version`.
    pub(crate) file: String,
    /// The processors one asset entry each is listed for.
    pub(crate) arches: Vec<Arch>,
    /// The oldest system it runs on.
    pub(crate) minimum_os: &'static str,
}

/// The desktop installers of `version`, by platform: the universal macOS disk image and the
/// Windows x64 installer.
#[must_use]
pub(crate) fn installers(version: &str) -> [Installer; 2] {
    [
        Installer {
            platform: Platform::Macos,
            file: format!("Emberhut-{version}-macos-universal.dmg"),
            arches: vec![Arch::Arm64, Arch::X86_64],
            minimum_os: "13.0",
        },
        Installer {
            platform: Platform::Windows,
            file: format!("Emberhut-{version}-windows-x64.msi"),
            arches: vec![Arch::X86_64],
            minimum_os: "10.0.19041",
        },
    ]
}

/// The feed's file name for `platform`, `emberhut-desktop-<platform>.json`.
#[must_use]
pub(crate) fn feed_name(platform: &Platform) -> String {
    format!("{PRODUCT}-{platform}.json")
}

/// The absolute download URL of `file` on `tag`'s release.
#[must_use]
pub(crate) fn download_url(tag: &Tag, file: &str) -> String {
    format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/{file}",
        tag.name()
    )
}

/// The update key's secret half, and the public half derived from it.
pub(crate) struct UpdateKey {
    secret: SecretKey,
    public: PublicKey,
}

impl std::fmt::Debug for UpdateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateKey")
            .field("id", &key_id(self.public.keynum()))
            .finish_non_exhaustive()
    }
}

impl UpdateKey {
    /// Reads an unencrypted minisign secret key file. Nothing of its content is ever printed,
    /// not even in an error.
    ///
    /// # Errors
    ///
    /// Fails when the file can't be read or isn't an unencrypted minisign secret key.
    pub(crate) fn read(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading the update key {}", path.display()))?;
        let invalid = || {
            anyhow::anyhow!(
                "{} is not an unencrypted minisign secret key",
                path.display()
            )
        };
        let secret = SecretKeyBox::from_string(&text)
            .and_then(SecretKey::from_unencrypted_box)
            .map_err(|_| invalid())?;
        if secret.is_encrypted() {
            return Err(invalid());
        }
        let public = PublicKey::from_secret_key(&secret).map_err(|_| invalid())?;
        Ok(Self { secret, public })
    }

    /// A key from a pair made in memory.
    #[must_use]
    pub(crate) fn from_pair(pair: minisign::KeyPair) -> Self {
        Self {
            secret: pair.sk,
            public: pair.pk,
        }
    }

    /// The public half, base64, as the second line of a `minisign.pub` file.
    #[must_use]
    pub(crate) fn public_base64(&self) -> String {
        self.public.to_base64()
    }

    /// The key id, 16 hex digits as minisign prints it.
    #[must_use]
    pub(crate) fn id(&self) -> String {
        key_id(self.public.keynum())
    }

    /// A minisign signature file (as text) of what `reader` yields, with `comment` as its
    /// trusted comment.
    ///
    /// # Errors
    ///
    /// Fails when reading fails or the key doesn't sign.
    pub(crate) fn sign(&self, reader: impl io::Read, comment: &str) -> anyhow::Result<String> {
        let signature = minisign::sign(
            Some(&self.public),
            &self.secret,
            reader,
            Some(comment),
            Some(&format!(
                "signature from the Emberhut update key {}",
                self.id()
            )),
        )
        .context("signing with the update key")?;
        Ok(signature.to_string())
    }
}

/// A minisign key number as its id: the little-endian number in 16 uppercase hex digits.
#[must_use]
pub(crate) fn key_id(keynum: &[u8]) -> String {
    let mut bytes = [0; 8];
    for (to, from) in bytes.iter_mut().zip(keynum) {
        *to = *from;
    }
    format!("{:016X}", u64::from_le_bytes(bytes))
}

/// The SHA-256 and size of the file at `path`, read in chunks.
///
/// # Errors
///
/// Fails when the file can't be read.
pub(crate) fn digest(path: &Path) -> anyhow::Result<([u8; 32], u64)> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 << 10];
    let mut size = 0u64;
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hasher.finalize().into(), size))
}

/// `bytes` as lowercase hex.
#[must_use]
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// When the feed is written: its `published` instant and its release's `date`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Moment {
    /// `2026-11-28T10:00:00.000Z`.
    pub(crate) published: Timestamp,
    /// `2026-11-28`.
    pub(crate) date: Day,
}

impl Moment {
    /// Now, in UTC.
    #[must_use]
    pub(crate) fn now() -> Self {
        Self::at(jiff::Timestamp::now())
    }

    /// `at`, in UTC, to the millisecond, as the contract writes times.
    #[must_use]
    pub(crate) fn at(at: jiff::Timestamp) -> Self {
        Self {
            published: Timestamp::new(at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
            date: Day::new(at.strftime("%Y-%m-%d").to_string()),
        }
    }
}

/// What `release feed` writes and reads.
#[derive(Debug)]
pub(crate) struct FeedInput<'a> {
    /// The release's tag.
    pub(crate) tag: &'a Tag,
    /// The notes, exactly as the feed carries them.
    pub(crate) notes: &'a str,
    /// Every asset of the release, as files.
    pub(crate) assets: &'a [PathBuf],
    /// When.
    pub(crate) moment: Moment,
}

/// A file name: no directories, no spaces, nothing a URL or `SHA256SUMS` would need to escape.
fn plain_file_name(path: &Path) -> anyhow::Result<&str> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("{} has no file name", path.display()))?;
    ensure!(
        name.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b)),
        "the asset name `{name}` has characters a download URL or SHA256SUMS would have to escape"
    );
    Ok(name)
}

/// Writes the feed of every desktop installer among the assets, then `SHA256SUMS` (of every
/// asset and feed) and its signature, into `out_dir`. Returns the files written.
///
/// # Errors
///
/// Fails when an asset is missing, named for another version or listed twice, the notes are
/// too long, or writing or signing fails.
pub(crate) fn write(
    input: &FeedInput<'_>,
    key: &UpdateKey,
    out_dir: &Path,
) -> anyhow::Result<Vec<PathBuf>> {
    let version = input.tag.version();
    ensure!(
        input.notes.len() <= super::notes::MAX_NOTES,
        "the notes are {} bytes; the feed holds {} at most",
        input.notes.len(),
        super::notes::MAX_NOTES
    );
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for path in input.assets {
        let name = plain_file_name(path)?;
        ensure!(
            name.contains(&version),
            "the asset {name} doesn't name the version {version}"
        );
        ensure!(
            name != SUMS && name != SUMS_SIGNATURE && !name.starts_with(&format!("{PRODUCT}-")),
            "{name} is written by `release feed`; don't pass it in"
        );
        ensure!(path.is_file(), "the asset {} is missing", path.display());
        ensure!(
            files.iter().all(|(known, _)| known != name),
            "the asset {name} is listed twice"
        );
        files.push((name.to_owned(), path.clone()));
    }
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let mut written = Vec::new();
    for installer in installers(&version) {
        let Some((_, path)) = files.iter().find(|(name, _)| *name == installer.file) else {
            continue;
        };
        let feed = feed(input, &installer, path, key)?;
        let out = out_dir.join(feed_name(&installer.platform));
        let mut json = serde_json::to_string_pretty(&feed).context("writing the feed's JSON")?;
        json.push('\n');
        fs::write(&out, json).with_context(|| format!("writing {}", out.display()))?;
        crate::say(&format!("Wrote {}", out.display()));
        files.push((feed_name(&installer.platform), out.clone()));
        written.push(out);
    }

    files.sort();
    let mut sums = String::new();
    for (name, path) in &files {
        let (sha256, _) = digest(path)?;
        sums.push_str(&hex(&sha256));
        sums.push_str("  ");
        sums.push_str(name);
        sums.push('\n');
    }
    let sums_path = out_dir.join(SUMS);
    fs::write(&sums_path, &sums).with_context(|| format!("writing {}", sums_path.display()))?;
    let signature = key.sign(sums.as_bytes(), &sums_comment(input.tag))?;
    let signature_path = out_dir.join(SUMS_SIGNATURE);
    fs::write(&signature_path, signature)
        .with_context(|| format!("writing {}", signature_path.display()))?;
    crate::say(&format!(
        "Wrote {} and {} ({} files), signed with the update key {}",
        sums_path.display(),
        signature_path.display(),
        files.len(),
        key.id()
    ));
    written.push(sums_path);
    written.push(signature_path);
    Ok(written)
}

/// The trusted comment of `SHA256SUMS.minisig`.
#[must_use]
pub(crate) fn sums_comment(tag: &Tag) -> String {
    format!("emberhut {} {SUMS}", tag.version())
}

/// The feed of `installer`, the file at `path`, signed for each of its arches.
fn feed(
    input: &FeedInput<'_>,
    installer: &Installer,
    path: &Path,
    key: &UpdateKey,
) -> anyhow::Result<ReleaseFeed> {
    let (sha256, size) = digest(path)?;
    let version = input.tag.version();
    let build = Build::new(input.tag.build());
    let assets = installer
        .arches
        .iter()
        .map(|arch| {
            let comment = update_comment(
                PRODUCT,
                &installer.platform,
                arch,
                &version,
                build,
                &sha256,
                input.notes,
            );
            let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
            Ok(Asset {
                arch: arch.clone(),
                url: download_url(input.tag, &installer.file),
                size,
                sha256: hex(&sha256),
                signature: key.sign(file, &comment)?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(ReleaseFeed {
        format: FORMAT,
        product: PRODUCT.to_owned(),
        platform: installer.platform.clone(),
        published: input.moment.published.clone(),
        releases: vec![Release {
            version,
            build,
            date: input.moment.date.clone(),
            notes: input.notes.to_owned(),
            minimum_os: Some(installer.minimum_os.to_owned()),
            assets,
        }],
    })
}

/// Checks `signature` (a minisign signature file, as text) of the file at `path` with `public`
/// and requires `comment` as its trusted comment, the way the app's updater does.
fn verify_signature(
    public: &minisign_verify::PublicKey,
    path: &Path,
    signature: &str,
    comment: &str,
) -> anyhow::Result<()> {
    let signature = minisign_verify::Signature::decode(signature).map_err(|err| {
        anyhow::anyhow!("the signature of {} doesn't read: {err}", path.display())
    })?;
    let mut verifier = public.verify_stream(&signature).map_err(|err| {
        anyhow::anyhow!(
            "the signature of {} isn't by the expected key: {err}",
            path.display()
        )
    })?;
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buffer = vec![0; 64 << 10];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => verifier.update(&buffer[..read]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }
    verifier
        .finalize()
        .map_err(|err| anyhow::anyhow!("the signature of {} is invalid: {err}", path.display()))?;
    ensure!(
        signature.trusted_comment() == comment,
        "the signature of {} carries `{}`, not `{comment}`",
        path.display(),
        signature.trusted_comment()
    );
    Ok(())
}

/// Reads back what [`write()`] made in `dir` for `tag`, as the app would: every feed's shape,
/// URLs, sizes, hashes and signatures (by `public_key`, base64), and `SHA256SUMS` with its
/// signature. The assets are looked up in `dir` by name. `expect` names the platforms whose feed
/// must be there. Returns how many asset signatures were checked.
///
/// # Errors
///
/// Fails on the first thing that doesn't match.
pub(crate) fn verify(
    dir: &Path,
    tag: &Tag,
    public_key: &str,
    expect: &[Platform],
) -> anyhow::Result<usize> {
    let public = minisign_verify::PublicKey::from_base64(public_key.trim())
        .map_err(|err| anyhow::anyhow!("the public key doesn't read: {err}"))?;
    let version = tag.version();
    let mut checked = 0;
    for installer in installers(&version) {
        let path = dir.join(feed_name(&installer.platform));
        if !path.is_file() {
            ensure!(
                !expect.contains(&installer.platform),
                "{} is missing",
                path.display()
            );
            continue;
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let feed: ReleaseFeed = serde_json::from_str(&text)
            .with_context(|| format!("{} isn't a release feed", path.display()))?;
        ensure!(
            feed.format == FORMAT && feed.product == PRODUCT && feed.platform == installer.platform,
            "{} is format {} of {} for {}, not format {FORMAT} of {PRODUCT} for {}",
            path.display(),
            feed.format,
            feed.product,
            feed.platform,
            installer.platform
        );
        let [release] = feed.releases.as_slice() else {
            bail!(
                "{} lists {} releases, not one",
                path.display(),
                feed.releases.len()
            );
        };
        ensure!(
            release.version == version && release.build == Build::new(tag.build()),
            "{} is for {} build {}, not {version} build {}",
            path.display(),
            release.version,
            release.build,
            tag.build()
        );
        ensure!(
            release.minimum_os.as_deref() == Some(installer.minimum_os),
            "{}: minimumOs isn't {}",
            path.display(),
            installer.minimum_os
        );
        let arches: Vec<&Arch> = release.assets.iter().map(|asset| &asset.arch).collect();
        ensure!(
            arches == installer.arches.iter().collect::<Vec<_>>(),
            "{} lists the arches {arches:?}, not {:?}",
            path.display(),
            installer.arches
        );
        let file = dir.join(&installer.file);
        let (sha256, size) = digest(&file)?;
        for asset in &release.assets {
            ensure!(
                asset.url == download_url(tag, &installer.file),
                "{}: the {} asset's URL is {}",
                path.display(),
                asset.arch,
                asset.url
            );
            ensure!(
                asset.size == size && asset.sha256 == hex(&sha256),
                "{}: the {} asset's size or SHA-256 isn't {}'s",
                path.display(),
                asset.arch,
                installer.file
            );
            let comment = update_comment(
                PRODUCT,
                &installer.platform,
                &asset.arch,
                &release.version,
                release.build,
                &sha256,
                &release.notes,
            );
            verify_signature(&public, &file, &asset.signature, &comment)?;
            checked += 1;
        }
    }

    let sums_path = dir.join(SUMS);
    let sums = fs::read_to_string(&sums_path)
        .with_context(|| format!("reading {}", sums_path.display()))?;
    let signature = fs::read_to_string(dir.join(SUMS_SIGNATURE))
        .with_context(|| format!("reading {SUMS_SIGNATURE}"))?;
    verify_signature(&public, &sums_path, &signature, &sums_comment(tag))?;
    ensure!(!sums.is_empty(), "{SUMS} is empty");
    for line in sums.lines() {
        let (expected, name) = line
            .split_once("  ")
            .with_context(|| format!("{SUMS} has a malformed line: {line}"))?;
        let (sha256, _) = digest(&dir.join(name))?;
        ensure!(hex(&sha256) == expected, "{name} doesn't match {SUMS}");
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use emberhut_api::releases::{Arch, Platform, ReleaseFeed};

    use super::{FeedInput, Moment, UpdateKey, feed_name, installers, key_id, verify, write};
    use crate::release::tag::Tag;

    fn moment() -> Moment {
        Moment::at("2026-11-28T09:30:15.123456Z".parse().unwrap())
    }

    fn new_key() -> UpdateKey {
        UpdateKey::from_pair(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
    }

    /// A release's assets, as small files in `dir`.
    fn assets(dir: &Path, version: &str) -> Vec<PathBuf> {
        [
            format!("Emberhut-{version}-macos-universal.dmg"),
            format!("Emberhut-{version}-windows-x64.msi"),
            format!("emberhut-server-{version}-linux-x64.tar.gz"),
            format!("emberhut-server-{version}-windows-x64.zip"),
        ]
        .into_iter()
        .map(|name| {
            let path = dir.join(&name);
            fs::write(&path, format!("the bytes of {name}")).unwrap();
            path
        })
        .collect()
    }

    #[test]
    fn the_moment_is_utc_to_the_millisecond() {
        let at = moment();
        assert_eq!(at.published.as_str(), "2026-11-28T09:30:15.123Z");
        assert_eq!(at.date.as_str(), "2026-11-28");
    }

    #[test]
    fn a_key_id_is_its_number_little_endian() {
        assert_eq!(
            key_id(&[0x44, 0x47, 0x56, 0xAD, 0x3E, 0x31, 0x00, 0xFF]),
            "FF00313EAD564744"
        );
    }

    #[test]
    fn the_feeds_list_the_installers_and_verify_as_the_app_would() {
        let dir = tempfile::tempdir().unwrap();
        let tag = Tag::parse("v0.1.0-rc.1").unwrap();
        let assets = assets(dir.path(), "0.1.0-rc.1");
        let key = new_key();
        let notes = "- desktop: rooms open faster (#12)\n";
        let input = FeedInput {
            tag: &tag,
            notes,
            assets: &assets,
            moment: moment(),
        };
        let written = write(&input, &key, dir.path()).unwrap();
        assert_eq!(written.len(), 4, "{written:?}");

        let mac: ReleaseFeed = serde_json::from_str(
            &fs::read_to_string(dir.path().join("emberhut-desktop-macos.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mac.platform, Platform::Macos);
        assert_eq!(mac.product, "emberhut-desktop");
        assert_eq!(mac.published.as_str(), "2026-11-28T09:30:15.123Z");
        let release = &mac.releases[0];
        assert_eq!(release.version, "0.1.0-rc.1");
        assert_eq!(release.build.get(), 10001);
        assert_eq!(release.notes, notes);
        assert_eq!(release.minimum_os.as_deref(), Some("13.0"));
        let [arm, intel] = release.assets.as_slice() else {
            panic!("two assets")
        };
        assert_eq!((&arm.arch, &intel.arch), (&Arch::Arm64, &Arch::X86_64));
        assert_eq!(arm.url, intel.url);
        assert_eq!(
            arm.url,
            "https://github.com/emberhuthq/emberhut/releases/download/v0.1.0-rc.1/\
             Emberhut-0.1.0-rc.1-macos-universal.dmg"
        );
        assert!(arm.signature.contains(
            "trusted comment: emberhut-desktop macos arm64 0.1.0-rc.1 build 10001 sha256:"
        ));
        assert!(
            intel
                .signature
                .contains("trusted comment: emberhut-desktop macos x86_64 ")
        );

        let windows: ReleaseFeed = serde_json::from_str(
            &fs::read_to_string(dir.path().join("emberhut-desktop-windows.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            windows.releases[0].minimum_os.as_deref(),
            Some("10.0.19041")
        );
        assert_eq!(windows.releases[0].assets.len(), 1);
        assert!(
            windows.releases[0].assets[0]
                .signature
                .contains(" windows x86_64 0.1.0-rc.1 ")
        );

        let sums = fs::read_to_string(dir.path().join("SHA256SUMS")).unwrap();
        let names: Vec<&str> = sums.lines().map(|line| &line[66..]).collect();
        assert_eq!(
            names,
            [
                "Emberhut-0.1.0-rc.1-macos-universal.dmg",
                "Emberhut-0.1.0-rc.1-windows-x64.msi",
                "emberhut-desktop-macos.json",
                "emberhut-desktop-windows.json",
                "emberhut-server-0.1.0-rc.1-linux-x64.tar.gz",
                "emberhut-server-0.1.0-rc.1-windows-x64.zip",
            ]
        );

        let both = [Platform::Macos, Platform::Windows];
        assert_eq!(
            verify(dir.path(), &tag, &key.public_base64(), &both).unwrap(),
            3
        );
        let stranger = new_key();
        assert!(verify(dir.path(), &tag, &stranger.public_base64(), &both).is_err());
        let other = Tag::parse("v0.1.0-rc.2").unwrap();
        assert!(verify(dir.path(), &other, &key.public_base64(), &both).is_err());
    }

    #[test]
    fn a_changed_payload_or_notes_fail_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let tag = Tag::parse("v0.1.0").unwrap();
        let assets = assets(dir.path(), "0.1.0");
        let key = new_key();
        let input = FeedInput {
            tag: &tag,
            notes: "- a change (#1)\n",
            assets: &assets,
            moment: moment(),
        };
        write(&input, &key, dir.path()).unwrap();
        let public = key.public_base64();
        verify(dir.path(), &tag, &public, &[Platform::Macos]).unwrap();

        let feed_path = dir.path().join(feed_name(&Platform::Windows));
        let feed = fs::read_to_string(&feed_path).unwrap();
        fs::write(
            &feed_path,
            feed.replace("- a change (#1)", "- another change (#1)"),
        )
        .unwrap();
        let err = verify(dir.path(), &tag, &public, &[]).unwrap_err();
        assert!(
            format!("{err:#}").contains("SHA256SUMS") || format!("{err:#}").contains("carries"),
            "{err:#}"
        );

        fs::write(&feed_path, feed).unwrap();
        verify(dir.path(), &tag, &public, &[]).unwrap();
        fs::write(&assets[0], "other bytes").unwrap();
        assert!(verify(dir.path(), &tag, &public, &[]).is_err());
    }

    #[test]
    fn only_the_installers_present_get_a_feed() {
        let dir = tempfile::tempdir().unwrap();
        let tag = Tag::parse("v0.1.0").unwrap();
        let assets: Vec<PathBuf> = assets(dir.path(), "0.1.0")
            .into_iter()
            .filter(|path| !path.to_string_lossy().ends_with(".msi"))
            .collect();
        let key = new_key();
        let input = FeedInput {
            tag: &tag,
            notes: "",
            assets: &assets,
            moment: moment(),
        };
        write(&input, &key, dir.path()).unwrap();
        assert!(dir.path().join("emberhut-desktop-macos.json").is_file());
        assert!(!dir.path().join("emberhut-desktop-windows.json").is_file());
        let public = key.public_base64();
        verify(dir.path(), &tag, &public, &[Platform::Macos]).unwrap();
        let err = verify(dir.path(), &tag, &public, &[Platform::Windows]).unwrap_err();
        assert!(
            err.to_string()
                .contains("emberhut-desktop-windows.json is missing"),
            "{err}"
        );
    }

    #[test]
    fn assets_must_name_the_version_once() {
        let dir = tempfile::tempdir().unwrap();
        let tag = Tag::parse("v0.2.0").unwrap();
        let key = new_key();
        let wrong = assets(dir.path(), "0.1.0");
        let input = FeedInput {
            tag: &tag,
            notes: "",
            assets: &wrong,
            moment: moment(),
        };
        let err = write(&input, &key, dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("doesn't name the version 0.2.0"),
            "{err}"
        );

        let right = assets(dir.path(), "0.2.0");
        let twice = [right[0].clone(), right[0].clone()];
        let input = FeedInput {
            tag: &tag,
            notes: "",
            assets: &twice,
            moment: moment(),
        };
        let err = write(&input, &key, dir.path()).unwrap_err();
        assert!(err.to_string().contains("listed twice"), "{err}");
        assert_eq!(
            installers("0.2.0")[1].file,
            "Emberhut-0.2.0-windows-x64.msi"
        );
    }

    #[test]
    fn a_key_file_reads_back_and_its_text_stays_out_of_errors() {
        let dir = tempfile::tempdir().unwrap();
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let text = pair.sk.to_box(None).unwrap().to_string();
        let path = dir.path().join("update.key");
        fs::write(&path, &text).unwrap();
        let key = UpdateKey::read(&path).unwrap();
        assert_eq!(key.public_base64(), pair.pk.to_base64());
        assert!(!format!("{key:?}").contains(&text[40..60]));

        let bad = dir.path().join("bad.key");
        fs::write(&bad, "untrusted comment: x\nRWRsecretlookingbutnotakey\n").unwrap();
        let err = format!("{:#}", UpdateKey::read(&bad).unwrap_err());
        assert!(
            err.contains("not an unencrypted minisign secret key"),
            "{err}"
        );
        assert!(!err.contains("RWRsecret"), "{err}");
    }
}
