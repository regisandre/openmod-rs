//! A model's session after a check panicked while using it.

use openmod::models::onnx::lock;
use std::sync::Mutex;

#[test]
fn a_model_keeps_working_after_a_check_panicked_while_using_it() {
    let session = Mutex::new("session");
    let crashed = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _held = lock(&session);
                panic!("a bug in a check");
            })
            .join()
    });
    assert!(crashed.is_err(), "the check panicked");
    assert!(session.is_poisoned(), "the lock remembers the panic");
    assert_eq!(
        *lock(&session),
        "session",
        "the next check still gets the session"
    );
}
