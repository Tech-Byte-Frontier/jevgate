use crate::options::CheckArgs;
use anyhow::{Context, Result, ensure};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub upload_allow: Vec<String>,
    pub upload_deny: Vec<String>,
    pub generated: Vec<String>,
    pub tests: Vec<String>,
    pub context: Vec<PathBuf>,
    pub rules: Vec<String>,
    pub max_requests: Option<u32>,
    pub concurrency: Option<u32>,
    pub max_file_bytes: Option<u64>,
    pub max_context_bytes: Option<u64>,
}

pub struct ConfigContext {
    pub invocation_dir: PathBuf,
    pub root: PathBuf,
    pub config: Config,
}

impl ConfigContext {
    pub fn discover() -> Result<Self> {
        let invocation_dir = std::env::current_dir()?.canonicalize()?;
        let root = repository_root(&invocation_dir);
        let file = root.join("jevgate.toml");
        let config = if file.exists() {
            toml::from_str(&std::fs::read_to_string(file)?).context("Invalid jevgate.toml")?
        } else {
            Config::default()
        };
        Ok(Self {
            invocation_dir,
            root,
            config,
        })
    }

    pub fn input_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.into()
        } else {
            self.invocation_dir.join(path)
        }
    }

    pub fn configure(&self, args: &mut CheckArgs) -> Result<()> {
        for path in &self.config.context {
            args.context.push(self.root.join(path));
        }
        if args.rules.is_empty() {
            args.rules = self.config.rules.clone();
        }
        if args.rules.is_empty() {
            args.rules = crate::maintainability::KEYS
                .iter()
                .map(|s| (*s).into())
                .collect();
        }
        for rule in &args.rules {
            ensure!(
                crate::maintainability::is_rule(rule),
                "Unknown rule: {rule}"
            );
        }
        // Configuration is a ceiling; CLI flags may narrow but cannot bypass upload budgets.
        if let Some(n) = self.config.max_requests {
            args.max_requests = Some(args.max_requests.map_or(n, |limit| limit.min(n)));
        }
        if let Some(n) = self.config.concurrency {
            ensure!(
                (1..=16).contains(&n),
                "Concurrency must be between 1 and 16"
            );
            args.concurrency = args.concurrency.min(n);
        }
        if let Some(n) = self.config.max_file_bytes {
            args.max_file_bytes = args.max_file_bytes.min(n);
        }
        if let Some(n) = self.config.max_context_bytes {
            args.max_context_bytes = args.max_context_bytes.min(n);
        }
        ensure!(
            args.max_requests != Some(0) && args.max_file_bytes > 0 && args.max_context_bytes > 0,
            "Budgets must be positive"
        );
        Ok(())
    }
}

pub fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

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

pub fn repository_root(invocation_dir: &Path) -> PathBuf {
    invocation_dir
        .ancestors()
        .find(|p| {
            p.join(".git/HEAD").is_file()
                || p.join(".git").is_file()
                || p.join("jevgate.toml").is_file()
        })
        .unwrap_or(invocation_dir)
        .to_path_buf()
}
