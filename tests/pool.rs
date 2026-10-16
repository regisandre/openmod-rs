//! Model sessions: as many checks run a model at once as it has sessions.

use openmod::pool::Pool;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn as_many_checks_run_a_model_at_once_as_it_has_sessions_and_the_next_one_waits() {
    let pool = Pool::new(vec![1, 2]);
    let first = pool.get();
    let second = pool.get();
    assert_ne!(*first, *second, "two checks hold two different sessions");
    assert!(pool.try_get().is_none(), "a third finds none free");
    let first_value = *first;
    let (sent, received) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let third = pool.get();
            sent.send(*third).unwrap();
        });
        assert!(
            received.recv_timeout(Duration::from_millis(200)).is_err(),
            "the third check waits while both sessions are busy"
        );
        drop(first);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)),
            Ok(first_value),
            "it takes the session the first check gave back"
        );
    });
    drop(second);
    assert!(pool.try_get().is_some(), "every session is back afterwards");
}

#[test]
fn a_session_comes_back_even_when_its_check_panics() {
    let pool = Pool::new(vec!["session"]);
    let crashed = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _session = pool.get();
                panic!("a bug in a check");
            })
            .join()
    });
    assert!(crashed.is_err(), "the check panicked");
    assert!(
        pool.try_get().is_some(),
        "the next check still finds the session"
    );
}
