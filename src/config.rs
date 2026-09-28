use crate::{
    catalog,
    options::{CheckArgs, FailOn},
};
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Globs of the paths that may be uploaded, including instruction files and context. Default: every path.
    pub upload_allow: Vec<String>,
    /// Globs never uploaded, even when allowed.
    pub upload_deny: Vec<String>,
    /// Globs of generated files, which are skipped, in addition to the built-in names.
    pub generated: Vec<String>,
    /// Globs of additional test files.
    pub tests: Vec<String>,
    /// Files always sent as related evidence, like `--context`.
    pub context: Vec<PathBuf>,
    /// A list selects rules; a table gives each group or rule a level. Default: the `default` group.
    pub rules: Rules,
    /// Ceiling on API attempts per invocation; flags can only lower it. Default: unlimited.
    pub max_requests: Option<u32>,
    /// Most simultaneous requests; flags can only lower it. JevGate sends at most 6 at once, so a higher value means 6. Default: 6 with a TypeSafe key, 3 with an OpenRouter or Vercel AI Gateway key.
    pub concurrency: Option<u32>,
    /// Files larger than this are reported as needs-context, never truncated. Default: 262144.
    pub max_file_bytes: Option<u64>,
    /// Ceiling on context bytes per request. Default: 32768.
    pub max_context_bytes: Option<u64>,
    /// The level for rules without their own, like `--fail-on`. Default: ["mature"], which fails only on the levels of a rule measured right at least 80% of the time on projects JevGate was never tuned on; `jevgate rules` shows them.
    pub fail_on: Vec<String>,
    /// Model, as the key's provider names it; a pinned version keeps results repeatable. `--model` overrides it. Default: `jev-1.13.0` with a TypeSafe key, `typesafe/jev-1.13` with an OpenRouter key, `typesafe-ai/jev` with a Vercel AI Gateway key.
    pub model: Option<String>,
    /// Cache lifetime in seconds for an alias, a model name without an x.y.z version such as `jev-latest`; pinned versions never expire. Default: 3600.
    pub cache_ttl_secs: Option<u64>,
    /// Judge tests, like `--include-tests`. Default: false.
    pub include_tests: bool,
    /// Gate levels for the files some paths match, such as report-only tooling.
    pub scope: Vec<Scope>,
    /// Custom questions: a yes/no question per convention, asked of each unit it names, whose yes is a finding. `.jevgate/questions/` holds one per file, named by its id.
    pub question: Vec<crate::custom::Spec>,
}

impl Config {
    /// The configuration in `file`, or the defaults when it does not exist
    /// and is not `required`.
    pub fn read(file: &Path, required: bool) -> Result<Self> {
        if !required && !file.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("Cannot read {}", file.display()))?;
        let config =
            toml::from_str(&text).with_context(|| format!("Invalid {}", file.display()))?;
        if let Some(notice) = written_before_mature(&text) {
            note!("jevgate: {notice}");
        }
        Ok(config)
    }
}

/// `[[scope]]`: gate levels for the files `paths` match. `fail_on` applies to
/// every rule there, and `rules` to single rules or groups. The last scope
/// that matches a file and addresses a rule wins; other files and rules keep
/// the levels set outside scopes.
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Globs of the files this scope applies to.
    pub paths: Vec<String>,
    /// The level for every rule in these files.
    #[serde(default)]
    pub fail_on: Vec<String>,
    /// Levels of single rules or groups in these files; `off` is not accepted (use `upload_deny`).
    #[serde(default)]
    pub rules: BTreeMap<String, Level>,
}

/// `rules = ["security"]` selects rules; a `[rules]` table sets each rule's or
/// group's gate level, or `"off"`, on top of the default group.
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
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
#[cfg_attr(test, derive(schemars::JsonSchema))]
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
                    anyhow!("Unknown level {name:?} for {target}; use review, consider, mature, uncertain, report or off")
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
    /// Custom questions, loaded once per process and kept for its life:
    /// rule keys are `&'static str` through planning and composition, as
    /// the built-in ones are compiled in.
    pub questions: &'static [crate::custom::Question],
}

impl ConfigContext {
    /// The repository around the working directory and its configuration:
    /// `file` when given (it must exist), else the root's jevgate.toml if any.
    /// Question files are read only with the repository's own configuration,
    /// so a change under review cannot edit a question to pass a policy that
    /// `--config` applies.
    pub fn discover(file: Option<&Path>) -> Result<Self> {
        Self::discover_in(&std::env::current_dir()?.canonicalize()?, file)
    }

    /// [`Self::discover`] from `invocation_dir`, an absolute canonical
    /// directory, such as the working directory an agent hook reports.
    pub fn discover_in(invocation_dir: &Path, file: Option<&Path>) -> Result<Self> {
        let invocation_dir = invocation_dir.to_path_buf();
        let root = repository_root(&invocation_dir);
        let (file, required) = match file {
            Some(file) => (invocation_dir.join(file), true),
            None => (root.join(crate::init::CONFIG_FILE), false),
        };
        let config = Config::read(&file, required)?;
        let directory = (!required).then(|| crate::custom::directory(&root));
        let questions =
            crate::custom::load(&root, (&file, &config.question), directory.as_deref())?;
        let filed = questions
            .iter()
            .find(|q| q.source.starts_with(crate::custom::DIRECTORY));
        if let Some(warning) = filed.and_then(|q| crate::custom::ignored(&root, &q.source)) {
            note!("{warning}");
        }
        Ok(Self {
            invocation_dir,
            root,
            config,
            questions: Box::leak(questions.into_boxed_slice()),
        })
    }

    /// The built-in rules and the custom questions.
    pub fn rules(&self) -> Vec<catalog::Rule> {
        catalog::with_custom(self.questions)
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
        args.questions = self.questions;
        args.model = args.model.take().or_else(|| self.config.model.clone());
        args.cache_ttl_secs = args.cache_ttl_secs.or(self.config.cache_ttl_secs);
        args.project = crate::docs::project_opening(
            &self.root,
            &crate::boundary::Boundary::new(&self.config)?,
        );
        self.configure_rules(args)?;
        self.configure_gate(args)?;
        self.configure_budgets(args)
    }

    /// Rules from `--rule`, else the configuration, else the `default` group,
    /// less `off` entries and `--skip-rule`. Every name must exist.
    fn configure_rules(&self, args: &mut CheckArgs) -> Result<()> {
        let rules = self.rules();
        let default = [catalog::DEFAULT_GROUP.to_string()];
        let mut enabled = BTreeSet::new();
        if !args.rules.is_empty() {
            enabled.extend(expand(&rules, &args.rules)?);
        } else {
            match &self.config.rules {
                Rules::List(names) if !names.is_empty() => enabled.extend(expand(&rules, names)?),
                Rules::List(_) => enabled.extend(expand(&rules, &default)?),
                Rules::Levels(levels) => {
                    enabled.extend(expand(&rules, &default)?);
                    for rule in &rules {
                        match most_specific(levels, rule) {
                            Some(level) if level.off() => enabled.remove(rule.key),
                            Some(_) => enabled.insert(rule.key),
                            None => false,
                        };
                    }
                }
            }
        }
        for skipped in expand(&rules, &args.skip_rules)? {
            enabled.remove(skipped);
        }
        args.rules = rules
            .iter()
            .filter(|rule| enabled.contains(rule.key))
            .map(|rule| rule.key.into())
            .collect();
        Ok(())
    }

    /// Each enabled rule's gate levels. The command line wins over the file;
    /// within each, a rule's own entry wins over its group's, then over the
    /// levels for every rule, then `mature`, or a custom question's own level.
    fn configure_gate(&self, args: &mut CheckArgs) -> Result<()> {
        let rules = self.rules();
        let cli = Levels::from_cli(&rules, &args.fail_on_specs)?;
        let file = self.file_levels(&rules)?;
        let fallback = [&cli.global, &file.global]
            .into_iter()
            .find(|levels| !levels.is_empty())
            .cloned()
            .unwrap_or_else(|| vec![FailOn::Mature]);
        args.fail_on = fallback.clone();
        args.rule_fail_on.clear();
        for rule in &rules {
            if !args.rules.iter().any(|r| r == rule.key) {
                continue;
            }
            let levels = cli
                .target(rule)
                .or_else(|| (!cli.global.is_empty()).then(|| cli.global.clone()))
                .or_else(|| file.target(rule))
                .unwrap_or_else(|| self.unaddressed(rule, &file.global, &fallback));
            if levels != fallback {
                args.rule_fail_on.insert(rule.key.into(), levels);
            }
        }
        self.configure_scopes(args, (&rules, &cli))
    }

    /// The levels of a rule no flag or `[rules]` entry addresses: `fail_on`
    /// of the file, else a custom question's own level, since whoever wrote
    /// and committed it chose where it fails, else `fallback`.
    fn unaddressed(
        &self,
        rule: &catalog::Rule,
        file: &[FailOn],
        fallback: &[FailOn],
    ) -> Vec<FailOn> {
        let question = self.questions.iter().find(|q| q.rule == rule.key);
        match question {
            Some(question) if file.is_empty() => question.default_levels(),
            _ => fallback.to_vec(),
        }
    }

    /// The levels each `[[scope]]` sets for the enabled rules. A flag that
    /// addresses a rule wins over every scope, as over the rest of the file.
    fn configure_scopes(
        &self,
        args: &mut CheckArgs,
        (all, cli): (&[catalog::Rule], &Levels),
    ) -> Result<()> {
        args.path_fail_on.clear();
        for scope in &self.config.scope {
            let levels = scope_levels(scope, all)?;
            let rules = all
                .iter()
                .filter(|rule| args.rules.iter().any(|r| r == rule.key))
                .filter(|rule| cli.global.is_empty() && cli.target(rule).is_none())
                .filter_map(|rule| {
                    let own = levels.target(rule);
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
    fn file_levels(&self, rules: &[catalog::Rule]) -> Result<Levels> {
        let mut levels = Levels::default();
        for name in &self.config.fail_on {
            levels
                .global
                .push(FailOn::parse(name).ok_or_else(|| anyhow!("Unknown fail_on value: {name}"))?);
        }
        if let Rules::Levels(entries) = &self.config.rules {
            for (target, level) in entries {
                expand(rules, std::slice::from_ref(target))?;
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
            ensure!(n > 0, "Concurrency must be at least 1");
            let most = crate::options::MAX_CONCURRENCY;
            args.concurrency = Some(args.concurrency.map_or(n.min(most), |flag| flag.min(n)));
        }
        cap_concurrency(args);
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

/// Lower a `--concurrency` above [`MAX_CONCURRENCY`] to it, saying so on
/// stderr: 0.25 accepted it up to 8, and a script valid then keeps working.
/// A higher `concurrency` in jevgate.toml, which 0.25 accepted too, is
/// lowered without a word when the file is read.
///
/// [`MAX_CONCURRENCY`]: crate::options::MAX_CONCURRENCY
fn cap_concurrency(args: &mut CheckArgs) {
    let most = crate::options::MAX_CONCURRENCY;
    if let Some(asked) = args.concurrency.filter(|n| *n > most) {
        note!(
            "jevgate: concurrency {asked} lowered to {most}, the most requests JevGate sends at once"
        );
        args.concurrency = Some(most);
    }
}

/// The levels one `[[scope]]` sets. `off` is not a gate level there: a rule
/// is judged for every file or none, and `upload_deny` keeps files out.
fn scope_levels(scope: &Scope, rules: &[catalog::Rule]) -> Result<Levels> {
    ensure!(!scope.paths.is_empty(), "Each [[scope]] needs paths");
    let mut levels = Levels::default();
    for name in &scope.fail_on {
        levels
            .global
            .push(FailOn::parse(name).ok_or_else(|| anyhow!("Unknown fail_on value: {name}"))?);
    }
    for (target, level) in &scope.rules {
        expand(rules, std::slice::from_ref(target))?;
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
    fn from_cli(rules: &[catalog::Rule], specs: &[crate::options::FailOnSpec]) -> Result<Self> {
        let mut levels = Self::default();
        for spec in specs {
            match &spec.target {
                Some(target) => {
                    expand(rules, std::slice::from_ref(target))?;
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

/// Keys of `rules` named by rule IDs, names, keys or groups; an unknown
/// name is an error.
fn expand(rules: &[catalog::Rule], names: &[String]) -> Result<Vec<&'static str>> {
    let mut keys = Vec::new();
    for name in names {
        let selected = catalog::select_in(rules, name).ok_or_else(|| unknown(rules, name))?;
        keys.extend(selected);
    }
    Ok(keys)
}

/// Why `name` names no rule, with the names that would: the custom
/// questions for a custom name, else the groups.
fn unknown(rules: &[catalog::Rule], name: &str) -> anyhow::Error {
    if name == catalog::CUSTOM_GROUP || catalog::custom(name) {
        let defined: Vec<&str> = rules
            .iter()
            .filter(|r| r.group == catalog::CUSTOM_GROUP)
            .map(|r| r.id)
            .collect();
        return match defined.as_slice() {
            [] => anyhow!(
                "Unknown custom question {name}: none is defined in jevgate.toml or {}",
                crate::custom::DIRECTORY
            ),
            _ => anyhow!(
                "Unknown custom question {name}; defined: {}",
                defined.join(", ")
            ),
        };
    }
    let custom = format!("{}/{name}", catalog::CUSTOM_GROUP);
    if rules.iter().any(|r| r.id == custom) {
        return anyhow!("Unknown rule or group: {name}; a custom question is named {custom}");
    }
    anyhow!(
        "Unknown rule or group: {name}; `jevgate rules` lists the rules (groups: {}, {}, {})",
        catalog::groups().join(", "),
        catalog::DEFAULT_GROUP,
        catalog::ALL_GROUP
    )
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

/// The groups `jevgate init` gave a `review` level before 0.26: every group
/// whose rules all ran by default.
const INIT_REVIEW_GROUPS: [&str; 2] = ["maintainability", "tests"];

/// What to tell a user whose jevgate.toml still holds the `[rules]` lines
/// `jevgate init` wrote before 0.26, such as `maintainability = "review"  #
/// file-organization, …`. They keep every review of those groups failing
/// the check and judge hardcoded values, which 0.26's measured default gate
/// and rules leave out, and most configurations were written that way. The
/// comment listing the group's rules tells them from a level set by hand,
/// so deleting it keeps the level without the notice.
fn written_before_mature(text: &str) -> Option<String> {
    let groups: Vec<&str> = INIT_REVIEW_GROUPS
        .into_iter()
        .filter(|group| {
            text.lines().any(|line| {
                line.trim()
                    .strip_prefix(group)
                    .is_some_and(|rest| rest.starts_with(" = \"review\"  # "))
            })
        })
        .collect();
    let (them, their) = match groups.len() {
        0 => return None,
        1 => ("it", "its comment"),
        _ => ("them", "their comments"),
    };
    let lines: Vec<String> = groups
        .iter()
        .map(|group| format!("`{group} = \"review\"`"))
        .collect();
    let hardcoded = if groups.contains(&"maintainability") {
        ", and hardcoded values is judged"
    } else {
        ""
    };
    Some(format!(
        "jevgate.toml keeps {} as `jevgate init` wrote {them} before 0.26: every {} review fails the check{hardcoded}. Delete {them} for the default rules and gate, which fails only on rule levels measured right at least 80% of the time; to keep {them}, delete {their} and this notice stops.",
        lines.join(" and "),
        groups.join(" and "),
    ))
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

    /// The configuration `toml_text` holds, in the working directory.
    fn context(toml_text: &str) -> Result<ConfigContext> {
        Ok(ConfigContext {
            invocation_dir: PathBuf::from("."),
            root: PathBuf::from("."),
            config: toml::from_str(toml_text)?,
            questions: crate::custom::parse(toml_text)?,
        })
    }

    fn configured(
        toml_text: &str,
        rules: &[&str],
        specs: &[(Option<&str>, FailOn)],
    ) -> Result<CheckArgs> {
        let context = context(toml_text)?;
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
    fn a_rule_is_named_by_its_id_its_name_or_its_key() {
        for rule in catalog::rules() {
            let (_, short) = rule.id.rsplit_once('/').unwrap();
            for name in [rule.id, short, rule.key] {
                assert_eq!(catalog::select(name).unwrap(), [rule.key], "{name}");
            }
        }
        let args = configured("", &["file-organization", "redundancy"], &[]).unwrap();
        assert_eq!(
            args.rules,
            [catalog::FILE_ORGANIZATION, catalog::TEST_REDUNDANCY]
        );
        let error = configured("", &["file-organisation"], &[]).unwrap_err();
        assert!(error.to_string().contains("`jevgate rules`"), "{error}");
    }

    #[test]
    fn the_levels_init_wrote_before_0_26_are_named_until_their_comments_go() {
        // What `jevgate init` wrote from 0.3 to 0.25, less its comments.
        let before = "[rules]\n\
            maintainability = \"review\"  # file-organization, function-simplification, hardcoded-values, shared-logic\n\
            tests = \"review\"  # test-value, test-redundancy\n\
            # security = \"consider\"  # injection, sensitive-data (opt-in)\n";
        let notice = written_before_mature(before).unwrap();
        assert!(
            notice.starts_with(
                "jevgate.toml keeps `maintainability = \"review\"` and `tests = \"review\"` as `jevgate init` wrote them before 0.26: every maintainability and tests review fails the check, and hardcoded values is judged. Delete them"
            ),
            "{notice}"
        );
        let tests_only = written_before_mature("tests = \"review\"  # test-value\n").unwrap();
        assert!(
            tests_only.contains("every tests review fails the check. Delete it"),
            "{tests_only}"
        );
        let project = crate::tests::Project::new();
        let (written, _) = crate::init::run(&project.0, false).unwrap();
        for kept in [
            "[rules]\nmaintainability = \"review\"\ntests = \"review\"\n".to_string(),
            "[rules]\nmaintainability = \"consider\"  # file-organization\n".to_string(),
            std::fs::read_to_string(written).unwrap(),
        ] {
            assert_eq!(written_before_mature(&kept), None, "{kept}");
        }
    }

    #[test]
    fn default_group_runs_when_nothing_is_configured() {
        let args = configured("", &[], &[]).unwrap();
        assert_eq!(args.rules, catalog::select(catalog::DEFAULT_GROUP).unwrap());
        assert!(!args.rules.iter().any(|r| r == catalog::HARDCODED_VALUES));
        assert_eq!(args.fail_on, [FailOn::Mature]);
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
    fn mature_is_the_default_and_a_level_like_the_others() {
        let args = configured("", &["default", "documentation"], &[]).unwrap();
        assert_eq!(args.levels(catalog::COMMENTS), [FailOn::Mature]);
        let names = |pairs: &[(&str, &str)]| -> BTreeMap<String, Vec<String>> {
            pairs
                .iter()
                .map(|(rule, level)| (rule.to_string(), vec![level.to_string()]))
                .collect()
        };
        assert_eq!(
            args.mature_level_names(),
            names(&[
                ("maintainability/function-simplification", "review"),
                ("documentation/agent-context", "consider")
            ])
        );
        let file = r#"
            fail_on = ["consider"]
            [rules]
            security = "mature"
            [[scope]]
            paths = ["scripts/**"]
            fail_on = ["mature", "uncertain"]
        "#;
        let args = configured(file, &["default", "security"], &[]).unwrap();
        assert_eq!(args.levels(catalog::SHARED_LOGIC), [FailOn::Consider]);
        assert_eq!(args.levels(catalog::INJECTION), [FailOn::Mature]);
        assert_eq!(
            args.levels_at(catalog::SHARED_LOGIC, Path::new("scripts/a.py")),
            [FailOn::Mature, FailOn::Uncertain]
        );
        assert_eq!(
            args.mature_level_names(),
            names(&[("maintainability/function-simplification", "review")]),
            "mature in a scope; injection has no mature level"
        );
        let flagged = configured(file, &["default"], &[(None, FailOn::Mature)]).unwrap();
        assert_eq!(flagged.levels(catalog::SHARED_LOGIC), [FailOn::Mature]);
        let explicit = configured("fail_on = [\"review\"]", &[], &[]).unwrap();
        assert!(explicit.mature_level_names().is_empty());
    }

    #[test]
    fn file_settings_apply_unless_a_flag_sets_them() {
        let file = "model = \"jev-latest\"\ncache_ttl_secs = 60\ninclude_tests = true\n";
        let args = configured(file, &[], &[]).unwrap();
        assert_eq!(
            (args.model(), args.cache_ttl_secs(), args.include_tests),
            ("jev-latest", 60, true)
        );
        let mut args = crate::tests::args();
        args.model = Some("jev-preview".into());
        args.cache_ttl_secs = Some(5);
        context(file).unwrap().configure(&mut args).unwrap();
        assert_eq!((args.model(), args.cache_ttl_secs()), ("jev-preview", 5));
        let defaults = configured("", &[], &[]).unwrap();
        assert_eq!(defaults.model(), crate::options::DEFAULT_MODEL);
    }

    const QUESTIONS: &str = r#"
        [[question]]
        id = "no-body-logs"
        question = "Does this function write a request body to a log?"
        unit = "function"
        [[question]]
        id = "owned-todos"
        question = "Does this comment hold a TODO without an owner or an issue?"
        unit = "comment"
        level = "consider"
    "#;

    #[test]
    fn custom_questions_are_selected_like_rules_by_id_group_default_and_all() {
        let custom = |args: &CheckArgs| -> Vec<String> {
            args.rules
                .iter()
                .filter(|r| catalog::custom(r))
                .cloned()
                .collect()
        };
        let both = ["custom/no-body-logs", "custom/owned-todos"];
        assert_eq!(custom(&configured(QUESTIONS, &[], &[]).unwrap()), both);
        for rules in [&["all"][..], &["default"], &["custom"]] {
            assert_eq!(custom(&configured(QUESTIONS, rules, &[]).unwrap()), both);
        }
        let named = configured(QUESTIONS, &["custom/owned-todos"], &[]).unwrap();
        assert_eq!(named.rules, ["custom/owned-todos"]);
        assert!(custom(&configured(QUESTIONS, &["security"], &[]).unwrap()).is_empty());
        let off = format!("{QUESTIONS}\n[rules]\n\"custom/no-body-logs\" = \"off\"\n");
        assert_eq!(
            custom(&configured(&off, &[], &[]).unwrap()),
            ["custom/owned-todos"]
        );
        let listed = format!("rules = [\"security\"]\n{QUESTIONS}");
        assert!(
            custom(&configured(&listed, &[], &[]).unwrap()).is_empty(),
            "a list selects rules, custom questions included"
        );
        for (rules, hint) in [
            (
                &["custom/nope"][..],
                "defined: custom/no-body-logs, custom/owned-todos",
            ),
            (
                &["no-body-logs"],
                "a custom question is named custom/no-body-logs",
            ),
        ] {
            let error = configured(QUESTIONS, rules, &[]).unwrap_err().to_string();
            assert!(error.contains(hint), "{error}");
        }
        let error = configured("", &["custom"], &[]).unwrap_err().to_string();
        assert!(error.contains("none is defined"), "{error}");
    }

    #[test]
    fn a_custom_question_fails_the_gate_at_its_level_unless_a_level_is_configured() {
        let args = configured(QUESTIONS, &[], &[]).unwrap();
        assert_eq!(args.levels("custom/no-body-logs"), [FailOn::Review]);
        assert_eq!(args.levels("custom/owned-todos"), [FailOn::Consider]);
        assert_eq!(
            args.rule_fail_on_names()["custom/owned-todos"],
            ["consider"]
        );
        assert_eq!(args.levels(catalog::SHARED_LOGIC), [FailOn::Mature]);
        let advisory = format!("fail_on = [\"none\"]\n{QUESTIONS}");
        let args = configured(&advisory, &[], &[]).unwrap();
        assert_eq!(args.levels("custom/owned-todos"), [FailOn::None]);
        let grouped = format!("{QUESTIONS}\n[rules]\ncustom = \"report\"\n");
        let args = configured(&grouped, &[], &[]).unwrap();
        assert_eq!(args.levels("custom/no-body-logs"), [FailOn::None]);
        let flagged = configured(QUESTIONS, &[], &[(None, FailOn::None)]).unwrap();
        assert_eq!(flagged.levels("custom/no-body-logs"), [FailOn::None]);
        let targeted = [(Some("custom/owned-todos"), FailOn::Review)];
        let args = configured(QUESTIONS, &[], &targeted).unwrap();
        assert_eq!(args.levels("custom/owned-todos"), [FailOn::Review]);
        let scoped = format!(
            "{QUESTIONS}\n[[scope]]\npaths = [\"scripts/**\"]\nrules = {{ \"custom/no-body-logs\" = \"report\" }}\n"
        );
        let args = configured(&scoped, &[], &[]).unwrap();
        let at = |path: &str| {
            args.levels_at("custom/no-body-logs", Path::new(path))
                .to_vec()
        };
        assert_eq!(
            (at("scripts/a.rs"), at("src/a.rs")),
            (vec![FailOn::None], vec![FailOn::Review])
        );
    }

    #[test]
    fn rule_lists_and_cli_rules_accept_groups_and_reject_unknown_names() {
        let args = configured("rules = [\"tests\"]", &[], &[]).unwrap();
        assert_eq!(
            args.rules,
            [catalog::TEST_VALUE, catalog::TEST_REDUNDANCY, catalog::LAWS]
        );
        let args = configured("rules = [\"tests\"]", &["shared_logic"], &[]).unwrap();
        assert_eq!(args.rules, [catalog::SHARED_LOGIC]);
        assert!(configured("", &["securty"], &[]).is_err());
        assert!(configured("[rules]\nmaintainability = \"sometimes\"\n", &[], &[]).is_err());
        assert!(configured("[rules]\nnothing = \"review\"\n", &[], &[]).is_err());
    }

    #[test]
    fn concurrency_follows_the_key_unless_set_and_is_at_most_six() {
        use crate::provider::Provider;
        // As a check runs: the configuration first, then the key's provider.
        let concurrency = |file: &str, flag: Option<u32>, provider| -> Result<u32> {
            let mut args = crate::tests::args();
            args.concurrency = flag;
            context(file)?.configure(&mut args)?;
            args.provider = provider;
            Ok(args.concurrency())
        };
        for (provider, default) in [
            (Provider::Typesafe, 6),
            (Provider::Openrouter, 3),
            (Provider::Vercel, 3),
        ] {
            let set = |file, flag| concurrency(file, flag, provider).unwrap();
            assert_eq!(set("", None), default, "{provider:?}");
            assert_eq!(set("concurrency = 5", None), 5, "the file sets it");
            assert_eq!(set("", Some(4)), 4, "the flag sets it");
            assert_eq!(set("concurrency = 2", Some(5)), 2, "the file caps the flag");
            for valid_in_0_25 in ["concurrency = 7", "concurrency = 8"] {
                assert_eq!(set(valid_in_0_25, None), 6, "{valid_in_0_25} means 6");
            }
            assert_eq!(
                set("concurrency = 8", Some(8)),
                6,
                "--concurrency 8 is lowered"
            );
        }
        assert!(concurrency("concurrency = 0", None, Provider::Typesafe).is_err());
    }
}
