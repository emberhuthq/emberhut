//! What the server has set up of what the assistant works with (`GET /api/setup`), item by
//! item, for the apps' first-run setup.

use emberhut_api::paths;
use emberhut_api::setup::SetupPage;

use crate::exchange;
use crate::{Client, Error};

impl Client {
    /// The setup checklist: each item and how it stands (`GET /api/setup`, with the session).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the route; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn setup(&self) -> Result<SetupPage, Error> {
        exchange::json(self.get(paths::SETUP, "")?)
    }
}
