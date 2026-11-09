mod service;
mod types;
mod validation;

pub use service::{ConfigService, remote_projection, validate_remote_projection_value};
pub use types::{
    ConfigChange, ConfigEffectFuture, ConfigEffectHooks, ConfigSnapshot, CredentialAvailability,
    PreparedConfigEffect, StoreMutationMetadata, WindowAccess,
};
