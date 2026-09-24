//! The separator boundary (FR-022, research.md, *Directory rename with a subtree*).
//!
//! `LIKE 'src%'` also matches `src-generated`. That is the whole hazard: it does not fail, it
//! silently rewrites rows nobody touched, and a spot check on two paths would not see it. The
//! returned row count is what makes the boundary assertable rather than sampled.

mod common;

use apex_shell::application::ports::workspace_cache::WorkspaceCache;
use apex_shell::domain::workspace::{RelPath, WorkspaceId};
use common::fake_cache::InMemoryCache;

fn ws() -> WorkspaceId {
    WorkspaceId("ws1".into())
}

fn rel(p: &str) -> RelPath {
    RelPath::parse(p).expect("a path")
}

fn seeded() -> InMemoryCache {
    let cache = InMemoryCache::default();
    common::fake_cache::seed(
        &cache,
        &ws(),
        &[
            "/src",
            "/src/main.rs",
            "/src/parser/expr.rs",
            "/src/parser/deep/token.rs",
            "/src-generated/build.rs",
            "/srcinfo.txt",
            "/other/a.rs",
        ],
    );
    cache
}

#[test]
fn renaming_a_directory_moves_it_and_everything_beneath_it() {
    let cache = seeded();
    let moved = cache
        .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
        .expect("rename");
    assert_eq!(
        moved, 4,
        "the directory and its three descendants, and nothing else"
    );
}

#[test]
fn a_sibling_sharing_a_prefix_is_untouched() {
    let cache = seeded();
    cache
        .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
        .expect("rename");
    let survivors = common::fake_cache::paths(&cache, &ws());
    assert!(
        survivors.contains(&"/src-generated/build.rs".to_string()),
        "{survivors:?}"
    );
    assert!(
        survivors.contains(&"/srcinfo.txt".to_string()),
        "{survivors:?}"
    );
    assert!(
        survivors.contains(&"/other/a.rs".to_string()),
        "{survivors:?}"
    );
}

#[test]
fn every_descendant_lands_under_the_new_prefix() {
    let cache = seeded();
    cache
        .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
        .expect("rename");
    let paths = common::fake_cache::paths(&cache, &ws());
    for expected in [
        "/syntax",
        "/syntax/main.rs",
        "/syntax/parser/expr.rs",
        "/syntax/parser/deep/token.rs",
    ] {
        assert!(
            paths.contains(&expected.to_string()),
            "missing {expected}: {paths:?}"
        );
    }
    assert!(
        !paths.iter().any(|p| p.starts_with("/src/")),
        "an old path survived: {paths:?}"
    );
}

#[test]
fn renaming_a_leaf_file_moves_exactly_one_row() {
    let cache = seeded();
    let moved = cache
        .rename_subtree(&ws(), &rel("/src/main.rs"), &rel("/src/app.rs"))
        .expect("rename");
    assert_eq!(moved, 1);
}

#[test]
fn renaming_something_absent_moves_nothing_rather_than_failing() {
    let cache = seeded();
    let moved = cache
        .rename_subtree(&ws(), &rel("/nope"), &rel("/also-nope"))
        .expect("rename");
    assert_eq!(moved, 0);
}

/// The same boundary, against SQL.
///
/// The fake proves the fake. `LIKE 'src%'` versus a separator-bounded range is a property of
/// the statement, and only the real store has one.
mod against_sqlite {
    use super::{rel, ws};
    use apex_shell::adapters::outbound::sqlite::schema;
    use apex_shell::adapters::outbound::sqlite::SqliteWorkspaceCache;
    use apex_shell::application::ports::workspace_cache::WorkspaceCache;
    use apex_shell::domain::workspace::{EntryKind, FsEntry, Location, RelPath, Workspace};

    fn entry(path: &str) -> FsEntry {
        let parsed = RelPath::parse(path).expect("a path");
        FsEntry {
            name: parsed.name().to_string(),
            kind: if path.ends_with(".rs") {
                EntryKind::File
            } else {
                EntryKind::Directory
            },
            size: 1,
            modified: 0,
        }
    }

    fn store() -> SqliteWorkspaceCache {
        let cache = SqliteWorkspaceCache::in_memory().expect("open");
        cache
            .migrate_to(schema::CURRENT_VERSION, &mut |_| {})
            .expect("migrate");
        let workspace = Workspace {
            id: ws(),
            name: "w".into(),
            location: Location::Local { base: "/ws".into() },
            last_opened_at: 0,
        };
        cache.register(&workspace, 0).expect("register");
        for (parent, children) in [
            ("/", vec!["src", "src-generated", "srcinfo.txt", "other"]),
            ("/src", vec!["main.rs", "parser"]),
            ("/src/parser", vec!["expr.rs"]),
            ("/src-generated", vec!["build.rs"]),
            ("/other", vec!["a.rs"]),
        ] {
            let base = if parent == "/" {
                String::new()
            } else {
                parent.to_string()
            };
            let items: Vec<FsEntry> = children
                .iter()
                .map(|c| entry(&format!("{base}/{c}")))
                .collect();
            cache
                .put_listing(&ws(), &rel(parent), &items)
                .expect("listing");
        }
        cache
    }

    fn paths(cache: &SqliteWorkspaceCache, parent: &str) -> Vec<String> {
        cache
            .list_children(&ws(), &rel(parent))
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn a_sibling_sharing_a_prefix_survives_the_rename() {
        let cache = store();
        let moved = cache
            .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
            .expect("rename");
        assert!(
            moved >= 3,
            "the directory and its descendants moved: {moved}"
        );

        let root = paths(&cache, "/");
        assert!(root.contains(&"src-generated".to_string()), "{root:?}");
        assert!(root.contains(&"srcinfo.txt".to_string()), "{root:?}");
        assert!(
            !root.contains(&"src".to_string()),
            "the old name is gone: {root:?}"
        );
        assert!(root.contains(&"syntax".to_string()), "{root:?}");
    }

    #[test]
    fn the_renamed_directory_keeps_its_own_parent_rather_than_becoming_its_own() {
        // Two formulas, not one. A descendant's parent_path carries the old prefix and can be
        // rewritten by substring arithmetic; the directory's own parent does not, and the same
        // formula would set it to itself.
        let cache = store();
        cache
            .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
            .expect("rename");
        let root = paths(&cache, "/");
        assert!(
            root.contains(&"syntax".to_string()),
            "it is still a child of the root: {root:?}"
        );
    }

    #[test]
    fn descendants_are_reachable_under_the_new_prefix() {
        let cache = store();
        cache
            .rename_subtree(&ws(), &rel("/src"), &rel("/syntax"))
            .expect("rename");
        let children = paths(&cache, "/syntax");
        assert!(children.contains(&"main.rs".to_string()), "{children:?}");
        assert!(children.contains(&"parser".to_string()), "{children:?}");
        let deep = paths(&cache, "/syntax/parser");
        assert!(deep.contains(&"expr.rs".to_string()), "{deep:?}");
    }
}
