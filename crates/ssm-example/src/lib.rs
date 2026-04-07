//! Example usage of the stactor crate.

pub mod relay_message;

use ssm::state_machine;

// A simple counter state machine demonstrating the basic pattern.
state_machine! {
    pub enum Counter {
        #[initial]
        Idle { count: u64 },
        Running { count: u64, started_at: u64 },
    }

    impl Counter {
        fn start(self: Idle) -> Running {
            CounterStateData::Running { count, started_at: 0 }
        }

        fn increment(self: Running) -> Running {
            CounterStateData::Running { count: count + 1, started_at }
        }

        fn stop(self: Running) -> Idle {
            CounterStateData::Idle { count }
        }
    }
}

#[cfg(all(test, not(feature = "async")))]
mod tests {
    use super::*;

    #[test]
    fn test_counter() {
        let counter = Counter::new();

        // Check initial state
        let state = counter.get_state();
        assert!(matches!(state, CounterStateData::Idle { count: 0 }));

        // Start and increment
        counter.start().unwrap();
        counter.increment().unwrap();
        counter.increment().unwrap();

        // Check running state
        let state = counter.get_state();
        assert!(matches!(state, CounterStateData::Running { count: 2, .. }));

        // Stop
        counter.stop().unwrap();
        let state = counter.get_state();
        assert!(matches!(state, CounterStateData::Idle { count: 2 }));
    }

    #[test]
    fn test_serialization() {
        let counter = Counter::new();
        counter.start().unwrap();
        counter.increment().unwrap();

        // Snapshot includes both state and context
        let snapshot = counter.snapshot();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("Running"));
        assert!(json.contains("\"count\":1"));

        // Deserialize
        let restored: CounterSnapshot = serde_json::from_str(&json).unwrap();
        assert!(matches!(restored.state, CounterStateData::Running { count: 1, .. }));
    }

    #[test]
    fn test_clone_and_concurrent() {
        let counter = Counter::new();
        let counter2 = counter.clone();

        // Both can mutate
        counter.start().unwrap();
        counter2.increment().unwrap();

        // Both see the same state
        let state1 = counter.get_state();
        let state2 = counter2.get_state();
        assert_eq!(state1, state2);
    }

    #[test]
    fn test_invalid_transition() {
        let counter = Counter::new();

        // Can't increment while Idle
        let err = counter.increment().unwrap_err();
        assert_eq!(err, IncrementError::InvalidState);

        // Can't stop while Idle
        let err = counter.stop().unwrap_err();
        assert_eq!(err, StopError::InvalidState);

        // Start works
        counter.start().unwrap();

        // Now increment works
        counter.increment().unwrap();

        // Can't start while Running
        let err = counter.start().unwrap_err();
        assert_eq!(err, StartError::InvalidState);
    }

    #[test]
    fn test_from_snapshot() {
        let counter = Counter::new();
        counter.start().unwrap();
        counter.increment().unwrap();
        counter.increment().unwrap();

        // Take snapshot
        let snapshot = counter.snapshot();

        // Restore from snapshot
        let restored = Counter::from_snapshot(snapshot);

        // Should be in same state
        let state = restored.get_state();
        assert!(matches!(state, CounterStateData::Running { count: 2, .. }));

        // Can continue from where we left off
        restored.increment().unwrap();
        let state = restored.get_state();
        assert!(matches!(state, CounterStateData::Running { count: 3, .. }));
    }
}
