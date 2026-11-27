//! Parties: two to five AI personas talking about the person's topic, turn by turn, while the
//! person watches it stream in (`GET /api/parties/{id}/live`). The person can jump in, steer,
//! pause, resume and stop; each party has a turn cap and a token cap, and its cost is counted at
//! list prices.
//!
//! The parties' contract is [`crate::party`]; this module has the two shapes only this server
//! answers: a party's page with its talk (`GET /api/parties/{id}`, [`PartyPage`]) and new caps
//! (`POST /api/parties/{id}/caps`, [`CapsRequest`]).

use serde::{Deserialize, Serialize};

use crate::party::{Party, PartyLine};

/// `GET /api/parties/{id}`: the party and its talk, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartyPage {
    /// The party.
    pub party: Party,
    /// What was said.
    pub messages: Vec<PartyLine>,
}

/// `POST /api/parties/{id}/caps`: new caps (turns 1 to 200, tokens 5,000 to 5,000,000); a cap
/// left out stays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CapsRequest {
    /// The turn cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<f64>,
    /// The token cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<f64>,
}
