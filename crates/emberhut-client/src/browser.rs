//! The assistant's browser (`/api/browser/*`): the live view's session, taking it over and
//! giving it back, its toolbar, and a file for the page.

use emberhut_api::browser::{BrowserInput, BrowserReply, BrowserSessionQuery, BrowserSessionReply};
use emberhut_api::paths;
use emberhut_api::uploads::UploadQuery;

use crate::exchange;
use crate::{Client, Error};

impl Client {
    /// The live view's address and the sign-in for what `query`'s device may do
    /// (`GET /api/browser/session?device=`): control while it drives, view only otherwise.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 503 with the server's reason when its machine has no live view;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn browser_session(
        &self,
        query: &BrowserSessionQuery,
    ) -> Result<BrowserSessionReply, Error> {
        exchange::json(self.get(paths::BROWSER_SESSION, &query.to_query())?)
    }

    /// Takes the browser over, gives it back, or drives its toolbar
    /// (`POST /api/browser/input`); the browser as it is after.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 409 with the server's reason (the toolbar before taking over);
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn browser_input(&self, input: &BrowserInput) -> Result<BrowserReply, Error> {
        exchange::json(self.post(paths::BROWSER_INPUT, input)?)
    }

    /// Puts `bytes`, a file called `query`'s name, into the field the page asks a file for
    /// (`POST /api/browser/upload?name=`, the raw file as the body); the browser as it is
    /// after.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 409 with the server's reason when the page isn't asking for a file;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn browser_upload(&self, query: &UploadQuery, bytes: &[u8]) -> Result<BrowserReply, Error> {
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let query = query.to_query();
        let response = self.send(|bearer| {
            let mut body = bytes;
            self.http
                .post_raw(paths::BROWSER_UPLOAD, &query, size, &mut body, bearer)
        })?;
        exchange::json(response)
    }
}
