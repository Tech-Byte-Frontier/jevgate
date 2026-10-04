//! What identifies a finding across edits, merges, selections and
//! renames: its fingerprint, the earlier ones a baseline may hold, and the
//! members a baseline matches by.
use super::*;

pub(super) fn fingerprint(rule: &str, plan: &FilePlan, identity: &str) -> String {
    hashed(rule, &plan.path, identity)
}

fn hashed(rule: &str, path: &Path, identity: &str) -> String {
    let separator = crate::schema::HASH_SEPARATOR;
    hash(format!("{rule}{separator}{}{separator}{identity}", path.display()).as_bytes())
}

/// The fingerprints of a unit identified by its rule, its file's path and
/// `identity` under the path a rename moved the file from.
fn renamed(rule: &str, plan: &FilePlan, identity: &str) -> Vec<String> {
    plan.previous
        .iter()
        .map(|before| hashed(rule, before, identity))
        .collect()
}

/// What a finding identified by its rule, path and `identity` is also
/// known by: its fingerprints under the path a rename moved its file from.
pub(super) fn renamed_only(rule: &str, plan: &FilePlan, identity: &str) -> Identity {
    Identity {
        aliases: renamed(rule, plan, identity),
        ..Default::default()
    }
}

/// A unit's fingerprint and what else its findings are known by. Most
/// units are identified by their path and content. A repeat is identified
/// by its copies alone, wherever it is reported; a file's outline by its
/// path, its members matched by similarity. Each of these, and a changed
/// hunk, also keeps the fingerprint JevGate 0.35 gave it, which a baseline
/// written then holds, and every unit of a renamed file its fingerprints
/// under the old path.
pub(super) fn known(plan: &FilePlan, unit: &UnitPlan) -> (String, Identity) {
    let rule = unit.rule;
    let (v1, members) = match &unit.detail {
        Detail::Pair { members, v1 } => (Some(v1.clone()), members.clone()),
        Detail::Outline { members, .. } => (Some(identity_of(members)), members.clone()),
        Detail::Custom(_, v1) => (v1.clone(), Vec::new()),
        _ => (None, Vec::new()),
    };
    let (now, mut aliases) = match &unit.detail {
        Detail::Pair { members, .. } => (copies(rule, members), moved_copies(rule, plan, members)),
        _ => (
            fingerprint(rule, plan, &unit.identity),
            renamed(rule, plan, &unit.identity),
        ),
    };
    aliases.extend(v1.iter().flat_map(|v1| renamed(rule, plan, v1)));
    let v1 = v1.map(|v1| fingerprint(rule, plan, &v1));
    (
        now,
        Identity {
            v1,
            aliases,
            members,
        },
    )
}

fn identity_of(members: &[String]) -> String {
    crate::units::identity(&members.iter().map(String::as_str).collect::<Vec<_>>())
}

/// A repeat's fingerprint: its rule and copies, whichever file reports it.
fn copies(rule: &str, members: &[String]) -> String {
    hashed(rule, Path::new(""), &identity_of(members))
}

/// A repeat's fingerprint with the copies in a renamed file under the path
/// it had before.
fn moved_copies(rule: &str, plan: &FilePlan, members: &[String]) -> Vec<String> {
    let Some(before) = &plan.previous else {
        return Vec::new();
    };
    let path = plan.path.to_string_lossy();
    let mut moved: Vec<String> = members
        .iter()
        .map(|member| match member.strip_prefix(&*path) {
            Some(rest) if rest.starts_with("::") || rest.starts_with('#') => {
                format!("{}{rest}", before.display())
            }
            _ => member.clone(),
        })
        .collect();
    moved.sort();
    vec![copies(rule, &moved)]
}
