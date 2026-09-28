use super::*;
use crate::tests::Project;

#[test]
fn a_root_below_the_git_top_level_sees_its_own_paths() {
    let project = Project::new();
    project.write("pkg/lib.rs", "fn a() {}\n");
    project.write("other/lib.rs", "fn a() {}\n");
    project.commit_all();
    project.write("pkg/lib.rs", "fn b() {}\n");
    project.write("other/lib.rs", "fn b() {}\n");
    project.write("pkg/new.rs", "fn c() {}\n");
    let changes = Changes::load(&project.0.join("pkg"), "HEAD").unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        [Path::new("lib.rs"), Path::new("new.rs")]
    );
}
