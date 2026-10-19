//! openmod: lightweight, CPU-only image and text moderation.

/// The version of this crate, for a program that embeds openmod and reports
/// which one it runs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod admin;
pub mod catalog;
pub mod config;
pub mod decision;
pub mod domains;
pub mod download;
pub mod edit;
pub mod engine;
pub mod eval;
pub mod extremism;
pub mod files;
pub mod image_input;
pub mod keys;
pub mod models;
pub mod normalize;
pub mod profanity;
pub mod qr;
pub mod rules;
pub mod server;
pub mod tools;
pub mod workflow;
