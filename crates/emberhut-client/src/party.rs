//! Party and the Dark room: the parties and their controls, a party's live stream, and the Dark
//! room's chat with its streamed answers.

use std::io::BufReader;

use emberhut_api::dark::{DarkEvent, DarkRequest, DarkRoom};
use emberhut_api::error::OkReply;
use emberhut_api::ids::PartyId;
use emberhut_api::party::{NewParty, PartiesPage, PartyAction, PartyEvent, PartyReply};
use emberhut_api::paths;

use crate::exchange;
use crate::live::{self, LiveEnd};
use crate::{Client, Error};

impl Client {
    /// The parties, the ready-made personas, the person's own and the models
    /// (`GET /api/parties`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without Party; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn parties(&self) -> Result<PartiesPage, Error> {
        exchange::json(self.get(paths::PARTIES, "")?)
    }

    /// Starts a new party (`POST /api/parties`); the party as the server made it.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no topic, a cast too small or too
    /// large, two personas of one name); as [`Client::parties`].
    pub fn start_party(&self, party: &NewParty) -> Result<PartyReply, Error> {
        exchange::json(self.post(paths::PARTIES, party)?)
    }

    /// One of the person's controls on a party (`POST /api/parties/{id}/{action}`); the party
    /// as it is after (none after a delete).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a party the server doesn't have, 400 with its reason for a
    /// blank line; as [`Client::parties`].
    pub fn act_on_party(&self, id: PartyId, action: &PartyAction) -> Result<PartyReply, Error> {
        let path = paths::party_action(id, action);
        let response = match action {
            PartyAction::Say(say) => self.post(&path, say)?,
            PartyAction::Pause | PartyAction::Resume | PartyAction::Stop | PartyAction::Delete => {
                self.post(&path, &serde_json::Map::new())?
            }
        };
        exchange::json(response)
    }

    /// Follows a party's live stream (`GET /api/parties/{id}/live`): its snapshot, then each
    /// change, to `on_event` until `stop` says so (asked as each event or the server's ping
    /// arrives) or the server closes it.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a party the server doesn't have, [`Error::Protocol`] for a
    /// stream that isn't an event stream; as [`Client::follow_live`].
    pub fn follow_party(
        &self,
        id: PartyId,
        stop: impl FnMut() -> bool,
        on_event: impl FnMut(PartyEvent),
    ) -> Result<LiveEnd, Error> {
        let response = self.get_stream(&paths::party_live(id), "")?;
        live::follow_events(
            BufReader::new(response.into_body()),
            self.or_stopped(stop),
            on_event,
        )
    }

    /// The Dark room (`GET /api/dark`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn dark(&self) -> Result<DarkRoom, Error> {
        exchange::json(self.get(paths::DARK, "")?)
    }

    /// Writes in the Dark room (`POST /api/dark`): the answer's events to `on_event` as it is
    /// written, until the server ends the stream. The answer is kept on the server even when
    /// the stream breaks off.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 for a blank message, 409 while an answer is being written, each
    /// with the server's words; the transport's errors when the stream breaks off.
    pub fn send_dark(
        &self,
        request: &DarkRequest,
        on_event: impl FnMut(DarkEvent),
    ) -> Result<(), Error> {
        let response = self.post_events(paths::DARK, request)?;
        live::follow_events(
            BufReader::new(response.into_body()),
            self.or_stopped(|| false),
            on_event,
        )
        .map(drop)
    }

    /// Deletes everything said in the Dark room (`DELETE /api/dark`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 409 while an answer is being written; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn clear_dark(&self) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.delete(paths::DARK)?)?;
        Ok(())
    }
}
