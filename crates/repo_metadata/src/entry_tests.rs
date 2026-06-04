use std::fs;

use ignore::gitignore::Gitignore;

use super::{matches_gitignores, Entry, IgnoredPathStrategy};
#[test]
fn test_git_path_filtering_allowlist() {
    use super::{is_commit_related_git_file, is_index_lock_file, should_ignore_git_path};
    use std::path::Path;

    // Non-git paths should not be ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/src/main.rs"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/README.md"
    )));

    // .git directory itself should be ignored
    assert!(should_ignore_git_path(Path::new("/home/user/project/.git")));

    // Allowlisted: commit-related files are NOT ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/HEAD"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/heads/main"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/heads/feature-branch"
    )));

    // Allowlisted: index.lock is NOT ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/index.lock"
    )));

    // Everything else in .git/ IS ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/index"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/config"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/COMMIT_EDITMSG"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/FETCH_HEAD"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/ORIG_HEAD"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/tags/v1.0"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/remotes/origin/main"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/objects/abc123"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/hooks/pre-commit"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/logs/HEAD"
    )));

    // Worktree paths: allowlisted patterns under .git/worktrees/<name>/
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/HEAD"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/index.lock"
    )));
    // Non-allowlisted worktree paths are still ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/index"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/COMMIT_EDITMSG"
    )));
    // worktrees dir itself (no content after worktree name) is ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt"
    )));

    // is_commit_related_git_file
    assert!(is_commit_related_git_file(Path::new("/repo/.git/HEAD")));
    assert!(is_commit_related_git_file(Path::new(
        "/repo/.git/refs/heads/main"
    )));
    assert!(is_commit_related_git_file(Path::new(
        "/repo/.git/worktrees/wt/HEAD"
    )));
    assert!(!is_commit_related_git_file(Path::new(
        "/repo/.git/index.lock"
    )));
    assert!(!is_commit_related_git_file(Path::new(
        "/repo/.git/refs/tags/v1"
    )));

    // is_index_lock_file
    assert!(is_index_lock_file(Path::new("/repo/.git/index.lock")));
    assert!(is_index_lock_file(Path::new(
        "/repo/.git/worktrees/wt/index.lock"
    )));
    assert!(!is_index_lock_file(Path::new("/repo/.git/HEAD")));
    assert!(!is_index_lock_file(Path::new("/repo/.git/index")));

    // Test Windows-style paths (only on Windows, as path parsing is platform-specific)
    #[cfg(windows)]
    {
        assert!(!should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\HEAD"
        )));
        assert!(!should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\index.lock"
        )));
        assert!(should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\index"
        )));
    }
}

#[test]
fn build_tree_marks_descendants_of_ignored_directory_as_ignored() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root_path = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::write(root_path.join(".gitignore"), "ignored-dir/\n").unwrap();
    fs::create_dir(root_path.join("ignored-dir")).unwrap();
    fs::write(root_path.join("ignored-dir").join("ignored-file.txt"), "").unwrap();

    let mut files = Vec::new();
    let mut gitignores = Vec::<Gitignore>::new();
    let tree = Entry::build_tree(
        &root_path,
        &mut files,
        &mut gitignores,
        None,
        10,
        0,
        &IgnoredPathStrategy::Include,
    )
    .unwrap();

    let Entry::Directory(root) = tree else {
        panic!("root should be a directory");
    };
    let ignored_dir = root
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-dir"))
        .unwrap();
    let Entry::Directory(ignored_dir) = ignored_dir else {
        panic!("ignored child should be a directory");
    };
    assert!(ignored_dir.ignored);

    let ignored_file = ignored_dir
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-file.txt"))
        .unwrap();
    assert!(ignored_file.ignored());
}

#[test]
fn lazy_loaded_ignored_directory_marks_loaded_children_as_ignored() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root_path = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::write(root_path.join(".gitignore"), "ignored-dir/\n").unwrap();
    fs::create_dir(root_path.join("ignored-dir")).unwrap();
    fs::write(root_path.join("ignored-dir").join("ignored-file.txt"), "").unwrap();

    let mut files = Vec::new();
    let mut gitignores = Vec::<Gitignore>::new();
    let mut tree = Entry::build_tree(
        &root_path,
        &mut files,
        &mut gitignores,
        None,
        10,
        0,
        &IgnoredPathStrategy::IncludeLazy,
    )
    .unwrap();

    let ignored_path = root_path.join("ignored-dir");
    let ignored_dir = tree.find_mut(&ignored_path).unwrap();
    let Entry::Directory(directory) = ignored_dir else {
        panic!("ignored child should be a directory");
    };
    assert!(directory.ignored);
    assert!(!directory.loaded);
    assert!(directory.children.is_empty());

    ignored_dir.load(&mut gitignores).unwrap();

    let Entry::Directory(directory) = ignored_dir else {
        panic!("ignored child should still be a directory");
    };
    assert!(directory.ignored);
    assert!(directory.loaded);

    let ignored_file = directory
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-file.txt"))
        .unwrap();
    assert!(ignored_file.ignored());
}

#[test]
fn should_watch_directory_in_git_path_prunes_non_allowlisted_subtrees() {
    use super::should_watch_directory_in_git_path;
    use std::path::Path;
    for path in [
        "/repo/.git",
        "/repo/.git/refs",
        "/repo/.git/refs/heads",
        "/repo/.git/refs/remotes",
        "/repo/.git/refs/remotes/origin",
        "/repo/.git/worktrees",
        "/repo/.git/worktrees/my-wt",
        "/repo/.git/worktrees/my-wt/refs",
        "/repo/.git/worktrees/my-wt/refs/heads",
    ] {
        assert!(
            should_watch_directory_in_git_path(Path::new(path)),
            "{path} should remain traversable so allowlisted git children stay reachable"
        );
    }

    for path in [
        "/repo/.git/objects",
        "/repo/.git/hooks",
        "/repo/.git/logs",
        "/repo/.git/info",
        "/repo/.git/lfs",
        "/repo/.git/refs/tags",
        "/repo/.git/worktrees/my-wt/objects",
        "/repo/.git/worktrees/my-wt/logs",
    ] {
        assert!(
            !should_watch_directory_in_git_path(Path::new(path)),
            "{path} should be pruned from recursive watcher registration"
        );
    }
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/objects/ab/blob"
    )));
    // The predicate is only consulted on directories during recursive registration;
    // file paths like `.git/HEAD` would never actually reach it, but the default
    // false return here documents that they're not treated as descend roots.
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/HEAD"
    )));
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/config"
    )));
}
#[test]
fn test_is_shared_git_ref() {
    use super::is_shared_git_ref;
    use std::path::Path;

    // Shared refs — broadcast to all repos
    assert!(is_shared_git_ref(Path::new("/repo/.git/refs/heads/main")));
    assert!(is_shared_git_ref(Path::new(
        "/repo/.git/refs/heads/feature"
    )));

    // Repo-specific — NOT shared
    assert!(!is_shared_git_ref(Path::new("/repo/.git/HEAD")));
    assert!(!is_shared_git_ref(Path::new("/repo/.git/index.lock")));

    // Worktree paths — NOT shared
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/worktrees/foo/HEAD"
    )));
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/worktrees/foo/refs/heads/main"
    )));

    // Other .git internals — NOT shared
    assert!(!is_shared_git_ref(Path::new("/repo/.git/refs/tags/v1")));
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/refs/remotes/origin/main"
    )));
    assert!(!is_shared_git_ref(Path::new("/repo/.git/config")));

    // Not a git path at all
    assert!(!is_shared_git_ref(Path::new("/repo/src/main.rs")));
}

#[test]
fn test_extract_worktree_git_dir() {
    use super::extract_worktree_git_dir;
    use std::path::{Path, PathBuf};

    // Standard worktree path extracts the per-worktree gitdir
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/foo/HEAD")),
        Some(PathBuf::from("/repo/.git/worktrees/foo"))
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/bar/index.lock")),
        Some(PathBuf::from("/repo/.git/worktrees/bar"))
    );

    // Non-worktree paths return None
    assert_eq!(extract_worktree_git_dir(Path::new("/repo/.git/HEAD")), None);
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/refs/heads/main")),
        None
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/src/main.rs")),
        None
    );

    // Edge case: not enough depth after worktrees/
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees")),
        None
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/foo")),
        None
    );
}

/// Writes a `.gitignore` with `content` at `root` and returns a [`Gitignore`]
/// rooted there. Uses only the repo-root gitignore (not the machine's global
/// gitignore) so tests are deterministic.
fn gitignore_rooted(root: &std::path::Path, content: &str) -> Gitignore {
    fs::write(root.join(".gitignore"), content).unwrap();
    let (gitignore, _) = Gitignore::new(root.join(".gitignore"));
    gitignore
}

#[test]
fn should_watch_prunes_gitignored_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir(root.join("node_modules")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, "node_modules/\n")];

    // Root and non-ignored dirs are watched; the gitignored dir is pruned.
    assert!(super::should_watch_repo_directory(&root, &gitignores, &[]));
    assert!(super::should_watch_repo_directory(
        &root.join("src"),
        &gitignores,
        &[]
    ));
    assert!(!super::should_watch_repo_directory(
        &root.join("node_modules"),
        &gitignores,
        &[]
    ));
    // Descendants of an ignored dir are also pruned (ancestor-aware), which is
    // what preserves the watcher's monotonicity invariant.
    assert!(!super::should_watch_repo_directory(
        &root.join("node_modules/foo"),
        &gitignores,
        &[]
    ));
}

#[test]
fn should_watch_descends_to_force_included_under_ignored_ancestor() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join(".agents/skills/test")).unwrap();
    fs::create_dir(root.join(".agents/other")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, ".agents/\n")];
    let force_included = vec![std::path::PathBuf::from(".agents/skills")];

    // The whole `.agents` subtree is gitignored, but we still descend along the
    // prefix to reach the force-included path, and into its subtree.
    assert!(super::should_watch_repo_directory(
        &root.join(".agents"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join(".agents/skills"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join(".agents/skills/test"),
        &gitignores,
        &force_included
    ));
    // A sibling ignored dir that is not force-included is still pruned.
    assert!(!super::should_watch_repo_directory(
        &root.join(".agents/other"),
        &gitignores,
        &force_included
    ));
}

#[test]
fn should_watch_handles_nested_ignored_ancestor_with_deeper_force_included() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::create_dir(root.join("a/b/other")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, "a/b/\n")];
    let force_included = vec![std::path::PathBuf::from("a/b/c")];

    // `a/b` is ignored but `a/b/c` is force-included: descend along the whole
    // prefix and into it, while pruning the ignored sibling.
    assert!(super::should_watch_repo_directory(
        &root.join("a"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join("a/b"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join("a/b/c"),
        &gitignores,
        &force_included
    ));
    assert!(!super::should_watch_repo_directory(
        &root.join("a/b/other"),
        &gitignores,
        &force_included
    ));
}

#[test]
fn should_watch_descends_dir_only_reinclude_negation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join("parentdir/sub")).unwrap();
    fs::write(root.join("parentdir/loose.txt"), "").unwrap();
    // Ignore the loose files in `parentdir` but re-include its subdirectories.
    let gitignores = vec![gitignore_rooted(&root, "parentdir/*\n!parentdir/*/\n")];

    // `parentdir` itself is not matched by `parentdir/*`, so we descend.
    assert!(super::should_watch_repo_directory(
        &root.join("parentdir"),
        &gitignores,
        &[]
    ));
    // The subdirectory is re-included by the directory-only negation, so it is
    // still watched even though `parentdir/*` matched it first.
    assert!(super::should_watch_repo_directory(
        &root.join("parentdir/sub"),
        &gitignores,
        &[]
    ));
    // The loose file remains gitignored (the negation is directory-only); the
    // emit predicate filters it, but `parentdir` stays watched for its subdirs.
    assert!(matches_gitignores(
        &root.join("parentdir/loose.txt"),
        false,
        &gitignores,
        true,
    ));
}

#[test]
fn should_watch_preserves_git_internal_allowlist() {
    // No gitignores / force-included paths needed: `.git` handling
    // short-circuits and is path-based, mirroring
    // `should_watch_directory_in_git_path`.
    let repo = std::path::Path::new("/home/user/project");
    assert!(super::should_watch_repo_directory(
        &repo.join(".git/refs/heads"),
        &[],
        &[]
    ));
    assert!(!super::should_watch_repo_directory(
        &repo.join(".git/objects"),
        &[],
        &[]
    ));
}
