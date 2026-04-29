//! # motux-fomod
//!
//! Parse and evaluate [FOMOD](https://fomod-docs.readthedocs.io/) mod installer configurations.
//!
//! FOMOD is an XML-based format used by mod managers (Mod Organizer 2, Vortex, etc.)
//! to define guided installation wizards for game mods.
//!
//! ## Quick start
//!
//! ```no_run
//! use motux_fomod::{ModuleConfig, Installer};
//!
//! let xml = std::fs::read_to_string("fomod/ModuleConfig.xml").unwrap();
//! let config = ModuleConfig::parse(&xml).unwrap();
//! let mut installer = Installer::new(config);
//!
//! // Present visible steps to the user, collect selections...
//! for (idx, step) in installer.visible_steps() {
//!     println!("Step {idx}: {}", step.name);
//! }
//!
//! // Record selections and resolve the install plan
//! installer.select(0, 0, vec![0]);
//! let plan = installer.resolve();
//! for op in &plan.operations {
//!     println!("{} -> {}", op.source, op.destination);
//! }
//! ```

pub mod condition;
pub mod config;
pub mod error;
pub mod info;
pub mod installer;

pub use condition::{EvalContext, Evaluate, FileState};
pub use config::ModuleConfig;
pub use error::{Error, Result};
pub use info::FomodInfo;
pub use installer::{
    CompletionStatus, FileConflict, FileConflictSource, FileOperation, FlagImpact, InstallPlan,
    Installer, SelectionError, ValidationHint,
};
