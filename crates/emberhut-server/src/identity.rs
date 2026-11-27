//! The machine's identity (ADR 0007, section 1): its Ed25519 key, kept in a file of its own,
//! and its name.
//!
//! The server makes the key from the operating system's randomness the first time it serves a
//! database, and writes it for the server's user only: to a temporary file in the same folder,
//! then moved into place without replacing a file that is already there, so two processes
//! starting at once end up with the same key. Commands (`pair`, `id`, `link`, `unlink`) only
//! read it.
//!
//! A new key makes a new machine that every device has to pair with again, so a key is never
//! replaced or made in place of one that went missing without being asked to. The database
//! remembers the id of the key it was served with ([`MACHINE_ID`]); serving it with no key, or
//! another machine's key (a mistyped `EMBERHUT_MACHINE_KEY`, a backup restored without its key),
//! stops the server with a message, until it is started once with `--new-machine-key`. A key
//! file that can't be read as a key, or that other users can read, stops it too.
//!
//! On Unix the file is made with mode `0600`, and one that the group or others can read is
//! refused. Elsewhere the file takes the permissions its folder gives new files, and they
//! aren't checked: keep the database's folder the server user's own.

use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use emberhut_api::connect::{MachineIdentity, ProbeAnswer, SignedLink};
use emberhut_api::directory::UnlinkRequest;
pub use emberhut_db::MACHINE_ID;
use emberhut_db::{DbError, SettingStore};
use emberhut_identity::{
    AccountId, Host, MachineId, MachineName, Nonce, PairingLink, PairingLinkError, ProbeKind,
    RandomError, Reached, RelayDomain, SECRET_KEY_LEN, SecretKey, UnixMillis,
};
use zeroize::Zeroizing;

/// The name of a machine whose host name can't be read or isn't a machine name.
const FALLBACK_NAME: &str = "Emberhut server";

/// Why the machine's key file couldn't be read or made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeyFileError {
    /// The file is there but couldn't be read.
    #[error("cannot read the machine's key from {}", .path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why not.
        #[source]
        source: io::Error,
    },
    /// The file isn't a key. It is left as it is.
    #[error(
        "{} is not a machine key ({SECRET_KEY_LEN} bytes; it has {len}). It is never replaced \
         on its own: restore it from a backup, or move it away to make this a new machine \
         (every device then pairs again, and the machine is linked to its account again)",
        .path.display()
    )]
    Malformed {
        /// The file.
        path: PathBuf,
        /// How many bytes it has.
        len: u64,
    },
    /// No key was there, and the operating system gave no randomness for a new one.
    #[error("cannot make the machine's key")]
    Random(#[from] RandomError),
    /// No key was there, and a new one couldn't be written.
    #[error("cannot write the machine's key to {}", .path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// Why not.
        #[source]
        source: io::Error,
    },
    /// Other users can read the file (Unix). It is left as it is.
    #[error(
        "other users can read the machine's key {} (mode {mode:o}): make it the server user's \
         only (chmod 600 {}); if someone else may have copied it, move it away and start the \
         server once with --new-machine-key to make this a new machine",
        .path.display(),
        .path.display()
    )]
    Shared {
        /// The file.
        path: PathBuf,
        /// Its permission bits.
        mode: u32,
    },
    /// Carrying the key, another machine's key was found where it goes. It is left as it is.
    #[error(
        "{} holds machine {found}'s key, not the one being carried ({carried}); it is left as it \
         is: move it away if that machine is gone, or put the new database in another folder",
        .path.display()
    )]
    Conflict {
        /// Where the key goes.
        path: PathBuf,
        /// The machine whose key is there.
        found: MachineId,
        /// The machine whose key is carried.
        carried: MachineId,
    },
    /// A command found no key: the server makes it when it first serves the database.
    #[error(
        "no machine key at {}: the server makes it when it first starts with this database; \
         start it once, or name the key with --machine-key (EMBERHUT_MACHINE_KEY)",
        .path.display()
    )]
    NoKey {
        /// Where it was looked for.
        path: PathBuf,
    },
}

/// Why the server won't serve with the key it found.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServingKeyError {
    /// The key file couldn't be read or made.
    #[error(transparent)]
    KeyFile(#[from] KeyFileError),
    /// The database couldn't say which key it was served with.
    #[error("the database failed")]
    Database(#[from] DbError),
    /// There is no key, but the database was served with one.
    #[error(
        "there is no machine key at {}, but this database was served as machine {id}. Restore \
         that machine's key there (from the backup the database came from), or point \
         EMBERHUT_MACHINE_KEY at it. To make this a new machine on purpose, whose devices all pair \
         again, start once with --new-machine-key",
        .path.display()
    )]
    Missing {
        /// Where the key was looked for.
        path: PathBuf,
        /// The machine the database was served as.
        id: String,
    },
    /// The key is another machine's than the one the database was served as.
    #[error(
        "the key at {} is machine {found}'s, but this database was served as machine {id}: \
         point EMBERHUT_MACHINE_KEY at this machine's key. To make this a new machine on purpose, \
         whose devices all pair again, start once with --new-machine-key",
        .path.display()
    )]
    NotThisMachine {
        /// The key file.
        path: PathBuf,
        /// The machine the database was served as.
        id: String,
        /// The machine the key file is.
        found: MachineId,
    },
}

/// The key to serve the database whose settings are `settings` with, from `path`: the one
/// there, if the database was served with it or with none; a new one, if there is none and the
/// database was never served with one. With `new_key`, the person asked for a new machine:
/// the key there (or a new one, if there is none) is taken whatever the database remembers.
/// The database then remembers the key's id.
///
/// # Errors
///
/// [`ServingKeyError::Missing`] or [`ServingKeyError::NotThisMachine`] when the key isn't the
/// one the database was served with (and `new_key` isn't set); the key file's and the
/// database's own errors.
pub async fn serving_key(
    path: &Path,
    settings: &SettingStore,
    new_key: bool,
) -> Result<SecretKey, ServingKeyError> {
    if new_key {
        tracing::warn!(
            path = %path.display(),
            "--new-machine-key: serving as whatever machine the key file holds (a new one if \
             there is none), whatever the database remembers; every device whose machine changes \
             pairs again. Use it for one start only, then remove it"
        );
    }
    let known = settings.get(&MACHINE_ID).await?;
    let key = match (load(path)?, known) {
        (Some(key), Some(id)) if !new_key && key.machine_id().to_string() != id => {
            return Err(ServingKeyError::NotThisMachine {
                path: path.to_owned(),
                id,
                found: key.machine_id(),
            });
        }
        (Some(key), _) => key,
        (None, Some(id)) if !new_key => {
            return Err(ServingKeyError::Missing {
                path: path.to_owned(),
                id,
            });
        }
        (None, _) => create(path)?,
    };
    let id = key.machine_id().to_string();
    if settings.get(&MACHINE_ID).await?.as_deref() != Some(id.as_str()) {
        settings.set(&MACHINE_ID, &id).await?;
    }
    Ok(key)
}

/// The machine's key, for a command run beside the server: read from `path`. Commands never
/// make one, so a command run with the wrong settings can't make a second machine.
///
/// # Errors
///
/// [`KeyFileError::NoKey`] when there is none, and the errors of [`load`].
pub fn existing_key(path: &Path) -> Result<SecretKey, KeyFileError> {
    load(path)?.ok_or_else(|| KeyFileError::NoKey {
        path: path.to_owned(),
    })
}

/// What [`carry_key`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CarriedKey {
    /// The key was copied to its new place.
    Copied,
    /// The same key was there already (the same file, or a copy of it).
    AlreadyThere,
    /// The old database's server never made a key: there is none to carry.
    NoKey,
}

/// Carries the machine's key from `from` (beside the old database) to `to` (beside the new
/// one), so the new database is served as the same machine: written for the server's user
/// only, to a temporary file then moved into place, never over another key.
///
/// # Errors
///
/// [`KeyFileError::Conflict`] when `to` holds another machine's key (it is left as it is), and
/// the errors of reading `from` and writing `to`.
pub fn carry_key(from: &Path, to: &Path) -> Result<CarriedKey, KeyFileError> {
    let Some(key) = load(from)? else {
        return Ok(CarriedKey::NoKey);
    };
    if let Some(there) = load(to)? {
        return if there == key {
            Ok(CarriedKey::AlreadyThere)
        } else {
            Err(KeyFileError::Conflict {
                path: to.to_owned(),
                found: there.machine_id(),
                carried: key.machine_id(),
            })
        };
    }
    match write_new(to, &key)? {
        Written::New => Ok(CarriedKey::Copied),
        Written::Existing(there) if *there == key => Ok(CarriedKey::AlreadyThere),
        Written::Existing(there) => Err(KeyFileError::Conflict {
            path: to.to_owned(),
            found: there.machine_id(),
            carried: key.machine_id(),
        }),
    }
}

/// Reads the machine's key from `path`; `None` when there is no file.
///
/// # Errors
///
/// [`KeyFileError::Read`] when the file can't be read, [`KeyFileError::Shared`] when other
/// users can read it (Unix), and [`KeyFileError::Malformed`] when it isn't a key.
pub fn load(path: &Path) -> Result<Option<SecretKey>, KeyFileError> {
    let read_error = |source| KeyFileError::Read {
        path: path.to_owned(),
        source,
    };
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(read_error(error)),
    };
    let meta = file.metadata().map_err(read_error)?;
    refuse_if_shared(path, &meta)?;
    let len = meta.len();
    if len != SECRET_KEY_LEN as u64 {
        return Err(KeyFileError::Malformed {
            path: path.to_owned(),
            len,
        });
    }
    let mut seed = Zeroizing::new([0_u8; SECRET_KEY_LEN]);
    file.read_exact(seed.as_mut()).map_err(read_error)?;
    Ok(Some(SecretKey::from_bytes(&seed)))
}

/// Makes a new key and writes it to `path`, unless another process wrote one there first:
/// then that one is the machine's.
fn create(path: &Path) -> Result<SecretKey, KeyFileError> {
    let key = SecretKey::generate()?;
    match write_new(path, &key)? {
        Written::Existing(there) => Ok(*there),
        Written::New => {
            tracing::info!(
                path = %path.display(),
                machine = %key.machine_id(),
                "made the machine's key"
            );
            Ok(key)
        }
    }
}

/// What [`write_new`] found.
enum Written {
    /// The key was written.
    New,
    /// Another key was there first, and was left as it is.
    Existing(Box<SecretKey>),
}

/// Writes `key` to `path` for the server's user only, through a temporary file in the same
/// folder moved into place, unless a file is there by then.
fn write_new(path: &Path, key: &SecretKey) -> Result<Written, KeyFileError> {
    let write_error = |source| KeyFileError::Write {
        path: path.to_owned(),
        source,
    };
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let mut temporary = private_temp_file(folder).map_err(write_error)?;
    temporary
        .write_all(key.to_bytes().as_ref())
        .map_err(write_error)?;
    temporary.as_file().sync_all().map_err(write_error)?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            return load(path)?
                .map(|there| Written::Existing(Box::new(there)))
                .ok_or_else(|| write_error(error.error));
        }
        Err(error) => return Err(write_error(error.error)),
    }
    // The new name is durable only once the folder is written too.
    #[cfg(unix)]
    File::open(folder)
        .and_then(|folder| folder.sync_all())
        .map_err(write_error)?;
    Ok(Written::New)
}

/// A temporary file in `folder` that only the server's user can read.
fn private_temp_file(folder: &Path) -> io::Result<tempfile::NamedTempFile> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(".machine.key.");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o600));
    }
    builder.tempfile_in(folder)
}

/// Refuses a key file that users other than the server's can read (Unix only; elsewhere the
/// folder's permissions apply, unchecked).
fn refuse_if_shared(path: &Path, meta: &std::fs::Metadata) -> Result<(), KeyFileError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(KeyFileError::Shared {
                path: path.to_owned(),
                mode,
            });
        }
    }
    #[cfg(not(unix))]
    let _ = (path, meta);
    Ok(())
}

/// The machine's name: the configured one, or the host name's first label (`Emberhut server`
/// when the host name can't be read or isn't a name).
#[must_use]
pub fn machine_name(configured: Option<&MachineName>) -> MachineName {
    configured
        .cloned()
        .or_else(|| {
            let host = host_name()?;
            MachineName::cut(host.split('.').next().unwrap_or_default()).ok()
        })
        .unwrap_or_else(fallback_name)
}

/// The name of a machine with no other.
#[expect(
    clippy::expect_used,
    reason = "a constant that is a valid name, which a test checks"
)]
fn fallback_name() -> MachineName {
    MachineName::cut(FALLBACK_NAME).expect("the fallback is a valid machine name")
}

/// The host name, as the operating system reports it.
fn host_name() -> Option<String> {
    #[cfg(unix)]
    {
        let uname = rustix::system::uname();
        uname.nodename().to_str().ok().map(str::to_owned)
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").ok()
    }
}

/// The machine: its key and its name. It signs what the machine says about itself.
pub struct Machine {
    key: SecretKey,
    name: MachineName,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("id", &self.id())
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Machine {
    /// The machine whose key is `key`, called `name`.
    #[must_use]
    pub const fn new(key: SecretKey, name: MachineName) -> Self {
        Self { key, name }
    }

    /// Its id.
    #[must_use]
    pub fn id(&self) -> MachineId {
        self.key.machine_id()
    }

    /// Its id and public key, as the pairing answer carries them.
    #[must_use]
    pub fn identity(&self) -> MachineIdentity {
        MachineIdentity::of(self.key.public_key())
    }

    /// Its name.
    #[must_use]
    pub const fn name(&self) -> &MachineName {
        &self.name
    }

    /// The pairing link for `code` over `routes`, the first [`PairingLink::MAX_ROUTES`] of
    /// them.
    ///
    /// # Errors
    ///
    /// [`PairingLinkError`] when the link would be too long for its QR code (very long routes).
    pub fn pairing_link(
        &self,
        code: &str,
        routes: &[String],
    ) -> Result<PairingLink, PairingLinkError> {
        let routes = routes
            .iter()
            .take(PairingLink::MAX_ROUTES)
            .cloned()
            .collect();
        PairingLink::new(self.id(), code, routes)
    }

    /// The machine's request to be linked to `account` at `relay_domain`, signed at `time`.
    #[must_use]
    pub fn sign_link(
        &self,
        account: AccountId,
        relay_domain: RelayDomain,
        time: UnixMillis,
    ) -> SignedLink {
        SignedLink::sign(&self.key, self.name.clone(), account, relay_domain, time)
    }

    /// The machine's answer to a probe with `nonce` and `kind` that reached it as `reached`,
    /// asking for `host`: the `emberhut probe v1` statement, signed.
    #[must_use]
    pub fn sign_probe(
        &self,
        nonce: Nonce,
        kind: &ProbeKind,
        reached: Reached,
        host: Host,
    ) -> ProbeAnswer {
        ProbeAnswer::sign(&self.key, nonce, kind, reached, host)
    }

    /// The machine's own request to leave its account at `relay_domain`, signed at `time`.
    #[must_use]
    pub fn sign_unlink(&self, relay_domain: RelayDomain, time: UnixMillis) -> UnlinkRequest {
        UnlinkRequest::sign(&self.key, relay_domain, time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key at `path`, made if there is none.
    fn load_or_create(path: &Path) -> Result<SecretKey, KeyFileError> {
        load(path)?.map_or_else(|| create(path), Ok)
    }

    #[test]
    fn a_key_is_made_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        assert!(load(&path).unwrap().is_none());
        let made = load_or_create(&path).unwrap();
        let again = load_or_create(&path).unwrap();
        assert_eq!(made, again);
        assert_eq!(std::fs::read(&path).unwrap().len(), SECRET_KEY_LEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let bits = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(bits & 0o777, 0o600);
        }
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file is left behind");
    }

    #[test]
    fn a_command_never_makes_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        let missing = existing_key(&path).unwrap_err();
        assert!(matches!(missing, KeyFileError::NoKey { .. }), "{missing:?}");
        assert!(missing.to_string().contains("--machine-key"), "{missing}");
        assert!(!path.exists());
        let made = load_or_create(&path).unwrap();
        assert_eq!(existing_key(&path).unwrap(), made);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_others_can_read_is_refused_and_left_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        load_or_create(&path).unwrap();
        for mode in [0o640, 0o604, 0o644] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            let refused = load(&path).unwrap_err();
            assert!(
                matches!(refused, KeyFileError::Shared { mode: got, .. } if got == mode),
                "{refused:?}"
            );
            assert!(refused.to_string().contains("chmod 600"), "{refused}");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(
            load(&path).unwrap().is_some(),
            "read-only for its owner is fine"
        );
    }

    #[tokio::test]
    async fn the_database_remembers_the_key_it_was_served_with() {
        let dir = tempfile::tempdir().unwrap();
        let url: emberhut_db::DatabaseUrl =
            format!("sqlite://{}", dir.path().join("emberhut.db").display())
                .parse()
                .unwrap();
        let db = emberhut_db::Database::connect(&url).await.unwrap();
        let settings = db.settings();
        let path = dir.path().join("machine.key");

        // First start: a key is made and remembered; later starts take it.
        let first = serving_key(&path, &settings, false).await.unwrap();
        assert_eq!(
            settings.get(&MACHINE_ID).await.unwrap(),
            Some(first.machine_id().to_string())
        );
        assert_eq!(serving_key(&path, &settings, false).await.unwrap(), first);

        // The key gone (a mistyped path, a restore without it): refused, and nothing is made.
        let elsewhere = dir.path().join("typo.key");
        let missing = serving_key(&elsewhere, &settings, false).await.unwrap_err();
        assert!(
            matches!(missing, ServingKeyError::Missing { .. }),
            "{missing:?}"
        );
        assert!(
            missing.to_string().contains("--new-machine-key"),
            "{missing}"
        );
        assert!(!elsewhere.exists());

        // Another machine's key: refused.
        let other = dir.path().join("other.key");
        load_or_create(&other).unwrap();
        let wrong = serving_key(&other, &settings, false).await.unwrap_err();
        assert!(
            matches!(wrong, ServingKeyError::NotThisMachine { .. }),
            "{wrong:?}"
        );

        // Asked for on purpose: a new machine, remembered from then on.
        let new = serving_key(&elsewhere, &settings, true).await.unwrap();
        assert_ne!(new, first);
        assert_eq!(
            serving_key(&elsewhere, &settings, false).await.unwrap(),
            new
        );
        assert!(serving_key(&path, &settings, false).await.is_err());
        db.close().await.unwrap();
    }

    #[test]
    fn a_malformed_key_file_is_an_error_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        std::fs::write(&path, b"not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let error = load_or_create(&path).unwrap_err();
        assert!(
            matches!(error, KeyFileError::Malformed { len: 9, .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("never replaced"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not a key");
    }

    #[test]
    fn a_missing_folder_is_a_write_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nowhere").join("machine.key");
        assert!(matches!(
            load_or_create(&path),
            Err(KeyFileError::Write { .. })
        ));
    }

    #[test]
    fn keys_made_at_once_agree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        let keys: Vec<SecretKey> = std::thread::scope(|scope| {
            let makers: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| load_or_create(&path).unwrap()))
                .collect();
            makers
                .into_iter()
                .map(|maker| maker.join().unwrap())
                .collect()
        });
        assert!(keys.iter().all(|key| *key == keys[0]));
        assert_eq!(load(&path).unwrap().unwrap(), keys[0]);
    }

    #[test]
    fn the_name_is_the_configured_one_or_the_host_s() {
        let desk: MachineName = "Desk".parse().unwrap();
        assert_eq!(machine_name(Some(&desk)), desk);
        let named = machine_name(None);
        assert!(!named.as_str().is_empty() && !named.as_str().contains('.'));
        assert_eq!(fallback_name().as_str(), FALLBACK_NAME);
    }

    #[test]
    fn the_machine_signs_links_its_id_names() {
        let machine = Machine::new(SecretKey::from_bytes(&[7; 32]), "Desk".parse().unwrap());
        assert_eq!(*machine.identity().id(), machine.id());
        let link = machine
            .pairing_link("ABCD-EFGH", &vec!["http://192.0.2.1:18990".to_owned(); 9])
            .unwrap();
        assert_eq!(link.routes().len(), PairingLink::MAX_ROUTES);
        assert_eq!(*link.id(), machine.id());
        let account = AccountId::new("user_1").unwrap();
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let time = UnixMillis::new(1_795_561_073_000).unwrap();
        let signed = machine.sign_link(account.clone(), domain.clone(), time);
        assert_eq!(signed.verify(&account, &domain), Ok(machine.id()));
        let unlink = machine.sign_unlink(domain.clone(), time);
        assert_eq!(unlink.verify(&machine.id(), &domain), Ok(()));
        assert!(!format!("{machine:?}").contains("key"));
    }
}
