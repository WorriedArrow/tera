//! stand-in for a core crate. run clippy with --features violations.

pub mod allowed;
#[cfg(feature = "violations")]
pub mod violations;
