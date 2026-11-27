//! The account directory over real HTTP on loopback, with a stand-in for Clerk's JWKS URL and
//! test-only keys: directory.md's routes and every row of its errors table, the token rules of
//! ADR 0007's section 5, the replay rules, CORS, and the gateway's interface.

mod cors;
mod gateway;
mod live;
mod routes;
mod statements;
mod support;
mod tokens;
