//! Kova core: the platform-independent part of the client.
//!
//! Parsing (share links, subscriptions, Clash and sing-box configs), the node model,
//! core capability planning, and config generation for Mihomo and Xray. No I/O lives
//! here, so everything is unit-testable and reusable on mobile.

pub mod clash;
pub mod compat;
pub mod gen_mihomo;
pub mod geodata;
pub mod gen_xray;
pub mod links;
pub mod model;
pub mod options;
pub mod singbox;
pub mod xray_json;
pub mod smart;
pub mod subscription;

pub use compat::{CoreKind, CorePreference, Plan};
pub use model::Node;
pub use options::RuntimeOptions;
