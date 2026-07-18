pub mod atomic_file;
pub mod config;
pub mod model;
pub mod resolver;
pub mod validation;

pub use atomic_file::write_file_atomically;
pub use config::{ConfigError, ConfigStore};
pub use model::*;
pub use resolver::{
    choose_power_plan, process_matches_enabled_profile, resolve_active_profiles,
    resolve_active_profiles_with_previous, ActiveProfile, DetectedProcess, PowerDecision,
};
pub use validation::{validate_config, ValidationCode, ValidationIssue};
