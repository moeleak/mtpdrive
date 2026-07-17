use super::*;

#[test]
fn records_are_incremental_and_queryable() {
    let store = LogStore::memory_only();
    let first = store.emit(LogLevel::Info, "test", "one");
    let second = store.emit(LogLevel::Warn, "test", "two");
    assert_eq!(store.after(first.id, 10), vec![second]);
}
