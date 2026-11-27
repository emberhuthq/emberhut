//! The optional emberhut.com account (ADR 0007, section 5): it lists the person's machines on
//! every device they sign in on, and lets a machine use the relay. It never lets anyone into a
//! machine: the device sessions made by pairing stay the only way in, and the account's sign-in
//! is kept apart from them.
//!
//! The account exists only in a build that turns it on ([`switch`]); a build from source has
//! none, and [`install`] then makes nothing, so no account view, request or task exists.
//!
//! - [`switch`]: the build-time settings.
//! - [`device_grant`], [`pkce`], [`oidc`]: signing in, with the device grant in every build and
//!   the system's web authentication session ([`web_session`]) in a build that claims the
//!   callback's associated domain.
//! - [`tokens`], [`vault`]: the tokens, kept in the login Keychain.
//! - [`directory`]: the directory's requests and refusals.
//! - [`session`]: the account's rules, with no GPUI; [`model`]: the entity the views observe
//!   (Settings' Account group and its sign-in sheet are `ui/settings/account.rs`).

pub(crate) mod device_grant;
pub(crate) mod directory;
pub(crate) mod http;
pub(crate) mod model;
pub(crate) mod oidc;
pub(crate) mod pkce;
pub(crate) mod session;
pub(crate) mod switch;
pub(crate) mod tokens;
pub(crate) mod vault;
pub(crate) mod web_session;

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, Global};

use self::http::UreqHttp;
use self::model::Account;
use self::session::AccountSession;
use self::switch::AccountSettings;
use crate::backend::Backend;
use crate::store::Store;

/// The account, once installed.
struct Installed(Entity<Account>);

impl Global for Installed {}

/// Makes the account, when this build has one: its tokens read from the Keychain, its machines
/// fetched while Settings shows them. `backend` is the server this app talks to, which signs
/// "Add this computer"'s link.
pub(crate) fn install(backend: Arc<dyn Backend>, store: &Entity<Store>, cx: &mut App) {
    let Some(settings) = AccountSettings::built_in() else {
        return;
    };
    let vault = vault::for_system(&settings.issuer);
    let session = Arc::new(AccountSession::new(
        settings,
        Arc::new(UreqHttp::new()),
        vault,
    ));
    let account = cx.new(|cx| Account::new(session, backend, store, cx));
    cx.set_global(Installed(account));
}

/// The account, in a build that has one.
#[must_use]
pub(crate) fn installed(cx: &App) -> Option<Entity<Account>> {
    cx.try_global::<Installed>()
        .map(|installed| installed.0.clone())
}
