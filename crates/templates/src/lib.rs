//! Dependency templates: prebuilt dependency directories mounted into sandboxes, never copied
//! into workspaces.
//!
//! A template lives in the templates library as `<name>/template.toml` (see [`TemplateManifest`]).
//! It is built inside the same base image the agents use, at exactly the path it will be mounted
//! at, because venvs and some native packages are not relocatable. A build is identified by a
//! hash of the manifest, the lockfiles and the image id, so a change to any of them marks it stale.

mod build;
mod manifest;
mod resolve;

pub use build::{BuildOutcome, TemplateBuilder, identity};
pub use manifest::*;
pub use resolve::{ResolvedTemplates, TemplateMount, resolve};

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
