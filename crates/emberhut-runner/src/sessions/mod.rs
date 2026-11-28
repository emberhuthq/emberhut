//! The sessions: each runs the agent in a folder, takes its turns one after the other, can be
//! interrupted or stopped, and reports every change as it happens.

pub mod engine;
pub mod folders;
pub mod live;
pub mod prompts;
