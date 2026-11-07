//! Neo: Opening a store: one store per file, a corrupt file set aside, reads refused once closed.

use std::fs;
use std::time::Duration;

use super::support::scratch;
use crate::db::sidecar;
use crate::{RunQuery, StoreConfig, StoreError, TraceStore};

#[tokio::test]
async fn a_second_store_on_the_same_file_fails_to_open_until_the_first_closes() {
    let (_dir, path) = scratch();
    let first = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let second = TraceStore::open(StoreConfig::new(&path)).await;
    assert!(
        matches!(&second, Err(StoreError::Locked { .. })),
        "the lock file keeps the file to one store: {:?}",
        second.err()
    );
    first.close(Duration::from_secs(5)).await;
    let reopened = TraceStore::open(StoreConfig::new(&path)).await;
    assert!(
        reopened.is_ok(),
        "closing releases the lock: {:?}",
        reopened.err()
    );
}

#[tokio::test]
async fn a_corrupt_file_is_set_aside_and_a_new_store_opens_in_its_place() {
    let (_dir, path) = scratch();
    let garbage = vec![b'x'; 8192];
    fs::write(&path, &garbage).unwrap();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    assert!(
        store
            .runs(RunQuery::recent(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "a new, empty store"
    );
    assert_eq!(
        fs::read(sidecar(&path, ".bak")).unwrap(),
        garbage,
        "the corrupt file is kept beside it"
    );
}

#[tokio::test]
async fn a_closed_store_refuses_reads() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let health = store.close(Duration::from_secs(5)).await;
    assert!(health.closed, "close reports a closed store");
    assert!(
        matches!(
            store.runs(RunQuery::recent(1)).await,
            Err(StoreError::Closed)
        ),
        "no read after close"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_alias_takes_the_same_lock_as_the_real_database() {
    use std::os::unix::fs as unix_fs;
    let (dir, path) = scratch();
    let first = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let alias = dir.path().join("alias.sqlite");
    unix_fs::symlink(&path, &alias).unwrap();
    assert!(
        matches!(
            TraceStore::open(StoreConfig::new(alias)).await,
            Err(StoreError::Locked { .. })
        ),
        "aliases share one lock"
    );
    first.close(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn dropping_the_last_store_closes_its_pools_before_releasing_its_lock() {
    use tokio::{task, time};
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    drop(store);
    time::timeout(Duration::from_secs(5), async {
        loop {
            let result = TraceStore::open(StoreConfig::new(&path)).await;
            if matches!(result, Err(StoreError::Locked { .. })) {
                task::yield_now().await;
                continue;
            }
            result.unwrap().close(Duration::from_secs(5)).await;
            break;
        }
    })
    .await
    .unwrap();
}
