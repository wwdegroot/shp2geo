//! Library crate: exposes the converter internals for integration tests
//! and embedding. The binary in `main.rs` is a thin wrapper.

pub mod convert;
pub mod error;
pub mod limits;
pub mod routes;
pub mod shapefile;
pub mod upload;
