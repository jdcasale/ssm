//! Async tests for stactor - only compiled with async feature.

#![cfg(feature = "async")]

use ssm::state_machine;

state_machine! {
    pub enum Counter {
        #[initial]
        Ready,
        Working,
    }

    impl Counter {
        fn start(self: Ready) -> Working {
            CounterStateData::Working
        }

        fn done(self: Working) -> Ready {
            CounterStateData::Ready
        }
    }
}

#[tokio::test]
async fn test_async_transitions() {
    let c = Counter::new();

    c.start().await.unwrap();

    let state = c.get_state().await;
    assert!(matches!(state, CounterStateData::Working));

    c.done().await.unwrap();

    let state = c.get_state().await;
    assert!(matches!(state, CounterStateData::Ready));
}

#[tokio::test]
async fn test_async_clone() {
    let c1 = Counter::new();
    let c2 = c1.clone();

    c1.start().await.unwrap();

    // c2 sees the change
    let state = c2.get_state().await;
    assert!(matches!(state, CounterStateData::Working));

    c2.done().await.unwrap();

    // c1 sees the change
    let state = c1.get_state().await;
    assert!(matches!(state, CounterStateData::Ready));
}

#[tokio::test]
async fn test_async_snapshot() {
    let c = Counter::new();
    c.start().await.unwrap();

    let snapshot = c.snapshot().await;
    assert!(matches!(snapshot.state, CounterStateData::Working));

    // Restore from snapshot
    let restored = Counter::from_snapshot(snapshot);
    let state = restored.get_state().await;
    assert!(matches!(state, CounterStateData::Working));
}

#[tokio::test]
async fn test_async_invalid_state() {
    let c = Counter::new();

    // Can't call done() from Ready
    let err = c.done().await.unwrap_err();
    assert_eq!(err, DoneError::InvalidState);

    // start() works
    c.start().await.unwrap();

    // Now done() works
    c.done().await.unwrap();
}

#[tokio::test]
async fn test_async_concurrent_tasks() {
    let c = Counter::new();

    // Spawn multiple tasks that interact with the same state machine
    let c1 = c.clone();
    let c2 = c.clone();

    let h1 = tokio::spawn(async move {
        c1.start().await.unwrap();
    });

    h1.await.unwrap();

    let h2 = tokio::spawn(async move {
        c2.done().await.unwrap();
    });

    h2.await.unwrap();

    let state = c.get_state().await;
    assert!(matches!(state, CounterStateData::Ready));
}
