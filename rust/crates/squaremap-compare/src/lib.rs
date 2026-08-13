//! Recording replay, parity comparison, and comparison reports.
//!
//! This crate implements the Task 14 comparison tooling: a bounded binary
//! recording stream ([`recording`]), semantic output-tree comparison
//! ([`compare`]), and a persistent JSON report schema with no-overwrite
//! semantics ([`report`]). The CLI in `main.rs` exposes the `replay`,
//! `compare-output`, and `report` subcommands.

#![forbid(unsafe_code)]

pub mod compare;
pub mod recording;
pub mod report;
pub mod benchmark;
