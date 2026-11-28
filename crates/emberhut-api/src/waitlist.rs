//! `/api/waitlist`: what the owner asked to hear about once it exists (renting a home in the
//! cloud). It is kept on this server until a central list exists: nothing is sent anywhere and
//! nothing is paid.

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

/// The list for a home rented in the cloud.
pub const CLOUD_HOME: &str = "cloud-home";

/// `GET /api/waitlist`: every list the server knows, and whether the owner is on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Waitlists {
    /// The lists.
    pub lists: Vec<Waitlist>,
}

/// One list (`PUT /api/waitlist/{list}` answers with it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Waitlist {
    /// Its id ([`CLOUD_HOME`]).
    pub id: String,
    /// The owner asked to hear about it.
    pub joined: bool,
    /// When they first asked.
    #[serde(default)]
    pub joined_at: Option<Timestamp>,
}
