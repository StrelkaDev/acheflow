//! Native provider execution and local account primitives. Trusted servers reuse
//! the bounded reactor with a separate server-owned account namespace.
#[cfg(not(target_arch = "wasm32"))]
pub mod access;
#[cfg(not(target_arch = "wasm32"))]
pub mod browser;
pub mod catalog;
pub mod direct_action;
pub mod local;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
mod stream;
pub mod transport;
#[cfg(not(target_arch = "wasm32"))]
mod vault;

#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;

#[cfg(not(target_arch = "wasm32"))]
pub mod server_account;

#[cfg(not(target_arch = "wasm32"))]
pub mod package;
