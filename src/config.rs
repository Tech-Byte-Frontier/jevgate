use crate::{
    catalog,
    options::{CheckArgs, FailOn, Provider},
};
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub upload_allow: Vec<String>,
    pub upload_deny: Vec<String>,
    pub generated: Vec<String>,
    pub tests: Vec<String>,
    pub context: Vec<PathBuf>,
    pub rules: Rules,
    pub max_requests: Option<u32>,
    pub concurrency: Option<u32>,
    pub max_file_bytes: Option<u64>,
    pub max_context_bytes: Option<u64>,
    /// Default `--fail-on` values when none are passed.
    pub fail_on: Vec<String>,
    /// The API provider when `--provider` is not passed.
    pub provider: Option<Provider>,
    /// The model when `--model` is not passed.
    pub model: Option<String>,
    /// Cache lifetime for model aliases when `--cache-ttl-secs` is not passed.
    pub cache_ttl_secs: Option<u64>,
    /// Judge tests as if `--include-tests` were passed.
    pub include_tests: bool,
    /// Gate levels for the files some paths match, such as report-only tooling.
    pub scope: Vec<Scope>,
}

/// `[[scope]]`: gate levels for the files `paths` match. `fail_on` applies to
/// every rule there, and `rules` to single rules or groups. The last scope
/// that matches a file and addresses a rule wins; other files and rules keep
/// the levels set outside scopes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub paths: Vec<String>,
    #[serde(default)]
    pub fail_on: Vec<String>,
    #[serde(default)]
    pub rules: BTreeMap<String, Level>,
}

/// `rules = ["security"]` selects rules; a `[rules]` table sets each rule's or
/// group's gate level, or `"off"`, on top of the default group.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Rules {
    List(Vec<String>),
    Levels(BTreeMap<String, Level>),
}

impl Default for Rules {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum Level {
    One(String),
    Many(Vec<String>),
}

impl Level {
    fn names(&self) -> Vec<&str> {
        match self {
            Self::One(name) => vec![name],
            Self::Many(names) => names.iter().map(String::as_str).collect(),
        }
    }

    fn off(&self) -> bool {
        self.names() == [OFF]
    }

    fn levels(&self, target: &str) -> Result<Vec<FailOn>> {
        self.names()
            .into_iter()
            .map(|name| {
                FailOn::parse(name).ok_or_else(|| {
                    anyhow!("Unknown level {name:?} for {target}; use review, consider, uncertain, report or off")
                })
            })
            .collect()
    }
}

const OFF: &str = "off";

pub struct ConfigContext {
    pub invocation_dir: PathBuf,
    pub root: PathBuf,
    pub config: Config,
}

impl ConfigContext {
    /// The repository around the working directory and its configuration:
    /// `file` when given (it must exist), else the root's jevgate.toml if any.
    pub fn discover(file: Option<&Path>) -> Result<Self> {
        let invocation_dir = std::env::current_dir()?.canonicalize()?;
        let root = repository_root(&invocation_dir);
        let (file, required) = match file {
            Some(file) => (invocation_dir.join(file), true),
            None => (root.join(crate::init::CONFIG_FILE), false),
        };
        let config = if required || file.exists() {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("Cannot read {}", file.display()))?;
            toml::from_str(&text).with_context(|| format!("Invalid {}", file.display()))?
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
        args.include_tests |= self.config.include_tests;
        args.provider = args.provider.or(self.config.provider);
        args.model = args.model.take().or_else(|| self.config.model.clone());
        args.cache_ttl_secs = args.cache_ttl_secs.or(self.config.cache_ttl_secs);
        self.configure_rules(args)?;
        self.configure_gate(args)?;
        self.configure_budgets(args)
    }

    /// Rules from `--rule`, else the configuration, else the `default` group,
    /// less `off` entries and `--skip-rule`. Every name must exist.
    fn configure_rules(&self, args: &mut CheckArgs) -> Result<()> {
        let mut enabled = BTreeSet::new();
        if !args.rules.is_empty() {
            enabled.extend(expand(&args.rules)?);
        } else {
            match &self.config.rules {
                Rules::List(names) if !names.is_empty() => enabled.extend(expand(names)?),
                Rules::List(_) => enabled.extend(expand(&[catalog::DEFAULT_GROUP.into()])?),
                Rules::Levels(levels) => {
                    enabled.extend(expand(&[catalog::DEFAULT_GROUP.into()])?);
                    for rule in catalog::rules() {
                        match most_specific(levels, &rule) {
                            Some(level) if level.off() => enabled.remove(rule.key),
                            Some(_) => enabled.insert(rule.key),
                            None => false,
                        };
                    }
                }
            }
        }
        for skipped in expand(&args.skip_rules)? {
            enabled.remove(skipped);
        }
        args.rules = catalog::keys()
            .into_iter()
            .filter(|key| enabled.contains(key))
            .map(Into::into)
            .collect();
        Ok(())
    }

    /// Each enabled rule's gate levels. The command line wins over the file;
    /// within each, a rule's own entry wins over its group's, then over the
    /// levels for every rule, then `review`.
    fn configure_gate(&self, args: &mut CheckArgs) -> Result<()> {
        let cli = Levels::from_cli(&args.fail_on_specs)?;
        let file = self.file_levels()?;
        let fallback = [&cli.global, &file.global]
            .into_iter()
            .find(|levels| !levels.is_empty())
            .cloned()
            .unwrap_or_else(|| vec![FailOn::Review]);
        args.fail_on = fallback.clone();
        args.rule_fail_on.clear();
        for rule in catalog::rules() {
            if !args.rules.iter().any(|r| r == rule.key) {
                continue;
            }
            let levels = cli
                .target(&rule)
                .or_else(|| (!cli.global.is_empty()).then(|| cli.global.clone()))
                .or_else(|| file.target(&rule))
                .unwrap_or_else(|| fallback.clone());
            if levels != fallback {
                args.rule_fail_on.insert(rule.key.into(), levels);
            }
        }
        self.configure_scopes(args, &cli)
    }

    /// The levels each `[[scope]]` sets for the enabled rules. A flag that
    /// addresses a rule wins over every scope, as over the rest of the file.
    fn configure_scopes(&self, args: &mut CheckArgs, cli: &Levels) -> Result<()> {
        args.path_fail_on.clear();
        for scope in &self.config.scope {
            let levels = scope_levels(scope)?;
            let rules = catalog::rules()
                .into_iter()
                .filter(|rule| args.rules.iter().any(|r| r == rule.key))
                .filter(|rule| cli.global.is_empty() && cli.target(rule).is_none())
                .filter_map(|rule| {
                    let own = levels.target(&rule);
                    let every = (!levels.global.is_empty()).then(|| levels.global.clone());
                    own.or(every).map(|l| (rule.key.to_string(), l))
                })
                .collect();
            args.path_fail_on.push(crate::options::PathLevels {
                matcher: crate::boundary::globs(&scope.paths)
                    .with_context(|| format!("Invalid scope paths {:?}", scope.paths))?,
                paths: scope.paths.clone(),
                rules,
            });
        }
        Ok(())
    }

    /// `fail_on` and the levels of the `[rules]` table.
    fn file_levels(&self) -> Result<Levels> {
        let mut levels = Levels::default();
        for name in &self.config.fail_on {
            levels
                .global
                .push(FailOn::parse(name).ok_or_else(|| anyhow!("Unknown fail_on value: {name}"))?);
        }
        if let Rules::Levels(entries) = &self.config.rules {
            for (target, level) in entries {
                expand(std::slice::from_ref(target))?;
                if !level.off() {
                    levels.targets.insert(target.clone(), level.levels(target)?);
                }
            }
        }
        Ok(levels)
    }

    /// Configuration is a ceiling; CLI flags may narrow but cannot bypass upload budgets.
    fn configure_budgets(&self, args: &mut CheckArgs) -> Result<()> {
        if let Some(n) = self.config.max_requests {
            args.max_requests = Some(args.max_requests.map_or(n, |limit| limit.min(n)));
        }
        if let Some(n) = self.config.concurrency {
            ensure!(
                (1..=crate::options::MAX_CONCURRENCY).contains(&n),
                "Concurrency must be between 1 and {}",
                crate::options::MAX_CONCURRENCY
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

/// The levels one `[[scope]]` sets. `off` is not a gate level there: a rule
/// is judged for every file or none, and `upload_deny` keeps files out.
fn scope_levels(scope: &Scope) -> Result<Levels> {
    ensure!(!scope.paths.is_empty(), "Each [[scope]] needs paths");
    let mut levels = Levels::default();
    for name in &scope.fail_on {
        levels
            .global
            .push(FailOn::parse(name).ok_or_else(|| anyhow!("Unknown fail_on value: {name}"))?);
    }
    for (target, level) in &scope.rules {
        expand(std::slice::from_ref(target))?;
        ensure!(
            !level.off(),
            "A [[scope]] cannot turn {target} off; use report, or upload_deny to skip the paths"
        );
        levels.targets.insert(target.clone(), level.levels(target)?);
    }
    Ok(levels)
}

/// Gate levels from one source: for every rule, and by rule or group name.
#[derive(Default)]
struct Levels {
    global: Vec<FailOn>,
    targets: BTreeMap<String, Vec<FailOn>>,
}

impl Levels {
    fn from_cli(specs: &[crate::options::FailOnSpec]) -> Result<Self> {
        let mut levels = Self::default();
        for spec in specs {
            match &spec.target {
                Some(target) => {
                    expand(std::slice::from_ref(target))?;
                    levels
                        .targets
                        .entry(target.clone())
                        .or_default()
                        .push(spec.level);
                }
                None => levels.global.push(spec.level),
            }
        }
        Ok(levels)
    }

    /// The levels of the entry that addresses `rule` most specifically.
    fn target(&self, rule: &catalog::Rule) -> Option<Vec<FailOn>> {
        most_specific(&self.targets, rule).cloned()
    }
}

/// Rule keys named by rule IDs, keys or groups; an unknown name is an error.
fn expand(names: &[String]) -> Result<Vec<&'static str>> {
    let mut keys = Vec::new();
    for name in names {
        let selected = catalog::select(name).ok_or_else(|| {
            anyhow!(
                "Unknown rule or group: {name} (groups: {}, {}, {})",
                catalog::groups().join(", "),
                catalog::DEFAULT_GROUP,
                catalog::ALL_GROUP
            )
        })?;
        keys.extend(selected);
    }
    Ok(keys)
}

/// The entry that addresses `rule` most specifically: its ID or key, its
/// group, then `default` or `all`.
fn most_specific<'a, T>(entries: &'a BTreeMap<String, T>, rule: &catalog::Rule) -> Option<&'a T> {
    entries
        .iter()
        .filter(|(name, _)| catalog::specificity(name, rule) > 0)
        .max_by_key(|(name, _)| catalog::specificity(name, rule))
        .map(|(_, value)| value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::FailOnSpec;

    fn configured(
        toml_text: &str,
        rules: &[&str],
        specs: &[(Option<&str>, FailOn)],
    ) -> Result<CheckArgs> {
        let context = ConfigContext {
            invocation_dir: PathBuf::from("."),
            root: PathBuf::from("."),
            config: toml::from_str(toml_text)?,
        };
        let mut args = crate::tests::args();
        args.rules = rules.iter().map(|r| r.to_string()).collect();
        args.fail_on.clear();
        args.fail_on_specs = specs
            .iter()
            .map(|(target, level)| FailOnSpec {
                target: target.map(Into::into),
                level: *level,
            })
            .collect();
        context.configure(&mut args)?;
        Ok(args)
    }

    #[test]
    fn default_group_runs_when_nothing_is_configured() {
        let args = configured("", &[], &[]).unwrap();
        assert_eq!(args.rules, catalog::select(catalog::DEFAULT_GROUP).unwrap());
        assert_eq!(args.fail_on, [FailOn::Review]);
        assert!(args.rule_fail_on.is_empty());
    }

    #[test]
    fn a_rule_entry_wins_over_its_group_and_off_disables_it() {
        let args = configured(
            r#"
            fail_on = ["consider"]
            [rules]
            maintainability = "review"
            "maintainability/hardcoded-values" = "off"
            tests = "report"
            test_value = ["review", "uncertain"]
            "#,
            &[],
            &[],
        )
        .unwrap();
        assert!(!args.rules.iter().any(|r| r == catalog::HARDCODED_VALUES));
        assert_eq!(args.fail_on, [FailOn::Consider]);
        assert_eq!(args.levels(catalog::SHARED_LOGIC), [FailOn::Review]);
        assert_eq!(args.levels(catalog::TEST_REDUNDANCY), [FailOn::None]);
        assert_eq!(
            args.levels("tests/value"),
            [FailOn::Review, FailOn::Uncertain]
        );
    }

    #[test]
    fn the_command_line_wins_over_the_file_and_targets_win_over_every_rule() {
        let file = "[rules]\nmaintainability = \"review\"\ntests = \"report\"\n";
        let args = configured(
            file,
            &[],
            &[
                (None, FailOn::Consider),
                (Some("tests/value"), FailOn::Uncertain),
            ],
        )
        .unwrap();
        assert_eq!(args.levels(catalog::SHARED_LOGIC), [FailOn::Consider]);
        assert_eq!(args.levels(catalog::TEST_REDUNDANCY), [FailOn::Consider]);
        assert_eq!(args.levels(catalog::TEST_VALUE), [FailOn::Uncertain]);
    }

    #[test]
    fn a_scope_sets_levels_for_its_paths_and_flags_win_over_it() {
        let file = r#"
            fail_on = ["consider"]
            [[scope]]
            paths = ["scripts/**", "tools/**"]
            fail_on = ["report"]
            [[scope]]
            paths = ["scripts/deploy/**"]
            rules = { security = "review" }
        "#;
        let rules = ["default", "security"];
        let args = configured(file, &rules, &[]).unwrap();
        let at = |args: &CheckArgs, rule: &str, path: &str| {
            args.levels_at(rule, Path::new(path)).to_vec()
        };
        assert_eq!(
            at(&args, catalog::SHARED_LOGIC, "src/a.ts"),
            [FailOn::Consider]
        );
        assert_eq!(
            at(&args, catalog::SHARED_LOGIC, "tools/a.ts"),
            [FailOn::None]
        );
        assert_eq!(
            at(&args, catalog::SHARED_LOGIC, "scripts/deploy/a.ts"),
            [FailOn::None],
            "the later scope does not address this rule"
        );
        assert_eq!(
            at(&args, "security/injection", "scripts/deploy/a.ts"),
            [FailOn::Review]
        );
        assert_eq!(
            at(&args, catalog::INJECTION, "scripts/a.ts"),
            [FailOn::None]
        );
        assert_eq!(args.path_fail_on_names()[1].rules.len(), 5);
        let flagged = configured(file, &rules, &[(None, FailOn::Consider)]).unwrap();
        assert_eq!(
            at(&flagged, catalog::SHARED_LOGIC, "scripts/a.ts"),
            [FailOn::Consider],
            "a flag wins over scopes as over the file"
        );
        for invalid in [
            "[[scope]]\npaths = []\nfail_on = [\"report\"]\n",
            "[[scope]]\npaths = [\"x/**\"]\nrules = { security = \"off\" }\n",
            "[[scope]]\npaths = [\"x/**\"]\nrules = { nothing = \"review\" }\n",
            "[[scope]]\npaths = [\"x/**\"]\nlevel = \"review\"\n",
        ] {
            assert!(configured(invalid, &[], &[]).is_err(), "{invalid}");
        }
    }

    #[test]
    fn file_settings_apply_unless_a_flag_sets_them() {
        let file = "model = \"jev-latest\"\ncache_ttl_secs = 60\ninclude_tests = true\n";
        let args = configured(file, &[], &[]).unwrap();
        assert_eq!(
            (args.model(), args.cache_ttl_secs(), args.include_tests),
            ("jev-latest", 60, true)
        );
        let context = ConfigContext {
            invocation_dir: PathBuf::from("."),
            root: PathBuf::from("."),
            config: toml::from_str(file).unwrap(),
        };
        let mut args = crate::tests::args();
        args.model = Some("jev-preview".into());
        args.cache_ttl_secs = Some(5);
        context.configure(&mut args).unwrap();
        assert_eq!((args.model(), args.cache_ttl_secs()), ("jev-preview", 5));
        let defaults = configured("", &[], &[]).unwrap();
        assert_eq!(defaults.provider(), Provider::TypeSafe);
        assert_eq!(defaults.model(), crate::options::DEFAULT_MODEL);
    }

    #[test]
    fn provider_selection_is_opt_in_and_controls_the_default_model() {
        let args = configured("provider = \"openrouter\"", &[], &[]).unwrap();
        assert_eq!(args.provider(), Provider::OpenRouter);
        assert_eq!(args.model(), crate::options::OPENROUTER_DEFAULT_MODEL);

        let context = ConfigContext {
            invocation_dir: PathBuf::from("."),
            root: PathBuf::from("."),
            config: toml::from_str("provider = \"openrouter\"").unwrap(),
        };
        let mut args = crate::tests::args();
        args.provider = Some(Provider::TypeSafe);
        context.configure(&mut args).unwrap();
        assert_eq!(args.provider(), Provider::TypeSafe);
        assert_eq!(args.model(), crate::options::DEFAULT_MODEL);
    }

    #[test]
    fn rule_lists_and_cli_rules_accept_groups_and_reject_unknown_names() {
        let args = configured("rules = [\"tests\"]", &[], &[]).unwrap();
        assert_eq!(args.rules, [catalog::TEST_VALUE, catalog::TEST_REDUNDANCY]);
        let args = configured("rules = [\"tests\"]", &["shared_logic"], &[]).unwrap();
        assert_eq!(args.rules, [catalog::SHARED_LOGIC]);
        assert!(configured("", &["securty"], &[]).is_err());
        assert!(configured("[rules]\nmaintainability = \"sometimes\"\n", &[], &[]).is_err());
        assert!(configured("[rules]\nnothing = \"review\"\n", &[], &[]).is_err());
    }
}
