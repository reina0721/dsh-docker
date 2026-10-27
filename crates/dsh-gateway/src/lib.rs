// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rust runtime for DSH Docker.
//!
//! `dsh-entrypoint` prepares the persistent home directory, supervises the DSH
//! web process, and serves the authenticated reverse proxy in one container.
//! The proxy replaces both the old Caddy gateway and the Node compatibility
//! proxy, preserving their external contract.

pub mod auth;
pub mod config;
pub mod home;
pub mod html;
pub mod proxy;
pub mod serve;
pub mod trust;
