//! Connect: the routes to a machine, the probe that checks one, the machine in the pairing
//! answer, and the link statement (`docs/api.md`, "Connect").
//!
//! Only emberhut's server has these routes and fields. A server without them is a **legacy host**:
//! its pairing answer has no `machine`, and the probe answers 404 or 405. Keys, ids, nonces,
//! signatures, names and the normal forms they sign are [`emberhut_identity`]'s types, written as
//! their text and checked when they are read. A key always comes with its own id: a message
//! whose `id` isn't its `publicKey`'s doesn't read.

mod binding;
mod link;
mod probe;
mod routes;

pub use link::{LinkError, LinkStatementRequest, SignedLink};
pub use probe::{ProbeAnswer, ProbeError, ProbeQuery, SshForward, VerifiedProbe};
pub use routes::{MachineIdentity, Route, RouteKind, RouteList};

/// `GET`: the machine's routes ([`RouteList`]), for a signed-in device.
pub const CONNECT: &str = "/api/connect";
/// `GET ?nonce=&kind=`: the probe ([`ProbeQuery`], [`ProbeAnswer`]); no session needed.
pub const PROBE: &str = "/api/connect/probe";
/// `POST`: a link statement ([`LinkStatementRequest`], [`SignedLink`]), on a TCP connection
/// from loopback only.
pub const LINK_STATEMENT: &str = "/api/connect/link-statement";

/// Gives a type that is checked as it is read the `OpenAPI` schema of its wire form, the plain
/// struct it reads from and writes to.
#[cfg(feature = "openapi")]
macro_rules! schema_of_wire {
    ($ty:ty, $wire:ty) => {
        impl ::utoipa::PartialSchema for $ty {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                <$wire as ::utoipa::PartialSchema>::schema()
            }
        }

        impl ::utoipa::ToSchema for $ty {
            fn schemas(
                schemas: &mut Vec<(
                    String,
                    ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema>,
                )>,
            ) {
                <$wire as ::utoipa::ToSchema>::schemas(schemas);
            }
        }
    };
}

#[cfg(feature = "openapi")]
pub(crate) use schema_of_wire;
