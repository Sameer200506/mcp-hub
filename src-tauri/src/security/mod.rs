//! Security-related building blocks.
//!
//! * [`permissions`] — the allow-list that bounds every write target;
//! * [`secrets`] — Windows Credential Manager integration (no insecure fallback).

pub mod permissions;
pub mod secrets;
