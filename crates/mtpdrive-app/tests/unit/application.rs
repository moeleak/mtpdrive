use super::SnapshotPoll;

#[test]
fn snapshot_poll_retries_after_a_failed_request_finishes() {
    let mut poll = SnapshotPoll::default();

    assert!(poll.begin());
    poll.finish();

    assert!(poll.begin());
}

#[test]
fn snapshot_poll_allows_only_one_request_at_a_time() {
    let mut poll = SnapshotPoll::default();

    assert!(poll.begin());
    assert!(!poll.begin());

    poll.finish();
    assert!(poll.begin());
}
