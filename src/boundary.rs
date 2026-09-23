//! Which files may be uploaded: the `upload_allow` and `upload_deny` globs of
//! `jevgate.toml`, relative to the project root.
use crate::config::Config;
use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

pub fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// The configured upload boundary; an empty allow list permits every path.
pub struct Boundary {
    allow: GlobSet,
    deny: GlobSet,
}
impl Boundary {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            allow: globs(&config.upload_allow)?,
            deny: globs(&config.upload_deny)?,
        })
    }
    pub fn permits(&self, path: &Path) -> bool {
        (self.allow.is_empty() || self.allow.is_match(path)) && !self.deny.is_match(path)
    }
}
