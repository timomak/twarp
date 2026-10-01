use super::*;
use futures::channel::oneshot;
use std::{cell::RefCell, rc::Rc, time::Duration};
use twarpui::{r#async::FutureExt as _, App, ModelHandle};
use virtual_fs::{Stub, VirtualFS};

fn model() -> LocalRepoMetadataModel {
    LocalRepoMetadataModel {
        repositories: HashMap::new(),
        lazy_loaded_paths: HashMap::new(),
        lazy_loading: LazyLoading::default(),
        watcher: None,
        emit_incremental_updates: false,
    }
}

fn next_root_result(
    app: &mut App,
    model: &ModelHandle<LocalRepoMetadataModel>,
) -> oneshot::Receiver<()> {
    let (sender, receiver) = oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    app.update(|ctx| {
        ctx.subscribe_to_model(model, move |_, event, _| {
            if matches!(
                event,
                RepositoryMetadataEvent::RepositoryUpdated { .. }
                    | RepositoryMetadataEvent::UpdatingRepositoryFailed { .. }
            ) {
                if let Some(sender) = sender.borrow_mut().take() {
                    let _ = sender.send(());
                }
            }
        });
    });
    receiver
}

#[test]
fn lazy_root_acquisition_is_pending_and_shared_until_background_completion() {
    VirtualFS::test("async_explorer_root", |dirs, mut fs| {
        fs.mkdir("tree")
            .with_files(vec![Stub::FileWithContent("tree/file.txt", "hello")]);
        let root = StandardizedPath::from_local_canonicalized(&dirs.tests().join("tree")).unwrap();
        App::test((), |mut app| async move {
            let handle = app.add_model(|_| model());
            let result = next_root_result(&mut app, &handle);
            handle.update(&mut app, |model, ctx| {
                model.request_index_lazy_loaded_path(&root, ctx);
                model.request_index_lazy_loaded_path(&root, ctx);
                assert!(matches!(
                    model.repository_state(&root),
                    Some(IndexedRepoState::Pending)
                ));
                assert_eq!(model.lazy_loaded_paths.get(&root), Some(&2));
                assert_eq!(model.lazy_loading.roots.len(), 1);
                let request = model.lazy_loading.roots[&root];
                model.remove_lazy_loaded_path(&root, ctx);
                model.remove_lazy_loaded_path(&root, ctx);
                assert!(model.repository_state(&root).is_none());
                model.request_index_lazy_loaded_path(&root, ctx);
                model.request_index_lazy_loaded_path(&root, ctx);
                assert_eq!(
                    model.lazy_loading.roots[&root], request,
                    "reopening joins the existing read"
                );
            });
            result
                .with_timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap();
            handle.update(&mut app, |model, ctx| {
                assert!(model
                    .get_repository(&root)
                    .unwrap()
                    .entry
                    .contains(&root.join("file.txt")));
                model.remove_lazy_loaded_path(&root, ctx);
                assert!(model.has_repository(&root));
                model.remove_lazy_loaded_path(&root, ctx);
                assert!(model.repository_state(&root).is_none());
                assert_eq!(model.lazy_loading.cached.len(), 1);
                assert_eq!(model.lazy_loading.epochs.get(&root), None);
            });
        });
    });
}

#[test]
fn missing_root_finishes_with_a_failure_instead_of_pending_forever() {
    VirtualFS::test("async_explorer_missing_root", |dirs, _| {
        let root = StandardizedPath::try_from_local(&dirs.tests().join("missing")).unwrap();
        App::test((), |mut app| async move {
            let handle = app.add_model(|_| model());
            let result = next_root_result(&mut app, &handle);
            handle.update(&mut app, |model, ctx| {
                model.request_index_lazy_loaded_path(&root, ctx)
            });
            result
                .with_timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap();
            handle.read(&app, |model, _| {
                assert!(matches!(
                    model.repository_state(&root),
                    Some(IndexedRepoState::Failed(_))
                ));
                assert_eq!(model.lazy_loading.roots.get(&root), None);
            });
        });
    });
}

#[test]
fn reopening_cached_root_revalidates_expanded_children() {
    VirtualFS::test("async_explorer_cached_root", |dirs, mut fs| {
        fs.mkdir("tree/src")
            .with_files(vec![Stub::FileWithContent("tree/src/old.txt", "old")]);
        let local_root = dirs.tests().join("tree");
        let root = StandardizedPath::from_local_canonicalized(&local_root).unwrap();
        let (mut cached, mut ignores) = read_root(&root, None).unwrap();
        cached
            .load_at_path(&root.join("src"), &mut ignores)
            .unwrap();
        std::fs::remove_file(local_root.join("src/old.txt")).unwrap();
        std::fs::write(local_root.join("src/new.txt"), "new").unwrap();
        let (refreshed, _) = read_root(&root, Some(cached)).unwrap();
        assert!(!refreshed.contains(&root.join("src/old.txt")));
        assert!(refreshed.contains(&root.join("src/new.txt")));
    });
}

#[test]
fn cached_roots_are_bounded_and_cancelled_requests_cannot_match_a_replacement_root() {
    let mut loading = LazyLoading::default();
    for index in 0..MAX_CACHED_ROOTS + 3 {
        let root = StandardizedPath::try_new(&format!("/root/{index}")).unwrap();
        loading.cache(
            root.clone(),
            FileTreeEntry::new_for_directory(Arc::new(root)),
        );
    }
    assert_eq!(loading.cached.len(), MAX_CACHED_ROOTS);
    assert_eq!(loading.cached.front().unwrap().0.as_str(), "/root/3");
    let root = StandardizedPath::try_new("/reopened").unwrap();
    let previous = loading.next_request();
    loading.roots.insert(root.clone(), previous);
    loading
        .directories
        .insert((root.clone(), root.join("src")), previous);
    loading.cancel(&root);
    let current = loading.next_request();
    loading.roots.insert(root.clone(), current);
    assert_ne!(loading.roots.get(&root), Some(&previous));
    assert_eq!(loading.directories.len(), 0);
}

#[test]
fn late_directory_read_cannot_overwrite_a_newer_tree_snapshot() {
    VirtualFS::test("async_explorer_stale_directory", |dirs, mut fs| {
        fs.mkdir("tree/src")
            .with_files(vec![Stub::FileWithContent("tree/src/old.txt", "old")]);
        let root = StandardizedPath::from_local_canonicalized(&dirs.tests().join("tree")).unwrap();
        let (initial, _) = read_root(&root, None).unwrap();
        let src = root.join("src");
        let marker = src.join("newer.txt");
        let mut replacement = initial.clone();
        replacement.insert_entry_at_path(
            Arc::new(src.clone()),
            Entry::Directory(crate::entry::DirectoryEntry {
                path: src.clone(),
                children: vec![Entry::File(crate::entry::FileMetadata::from_standardized(
                    marker.clone(),
                    false,
                ))],
                ignored: false,
                loaded: true,
            }),
        );
        App::test((), |mut app| async move {
            let handle = app.add_model(|_| model());
            handle.update(&mut app, |model, ctx| {
                model.insert_test_state(root.clone(), FileTreeState::from_file_tree_entry(initial));
                model.request_load_directory(&root, &src, ctx);
                assert!(model.has_pending_explorer_loads());
                model.insert_test_state(
                    root.clone(),
                    FileTreeState::from_file_tree_entry(replacement),
                );
            });
            async {
                while handle.read(&app, |model, _| model.has_pending_explorer_loads()) {
                    futures_lite::future::yield_now().await;
                }
            }
            .with_timeout(Duration::from_secs(5))
            .await
            .unwrap();
            handle.read(&app, |model, _| {
                let state = model.get_repository(&root).unwrap();
                assert!(state.entry.contains(&marker));
                assert!(!state.entry.contains(&src.join("old.txt")));
            });
        });
    });
}
