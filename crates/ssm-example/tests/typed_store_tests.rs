//! Tests for typed store! macro with handle-based typestate transitions.

use ssm::store;

// ============================================================================
// Map-based store tests
// ============================================================================

store! {
    pub MessageStore {
        messages: Map<u64> {
            states {
                Pending,
                InFlight { leader: u64, term: u64 },
                InDoubt { prev_leader: u64, prev_term: u64 },
                Acked,
            }

            transitions {
                mark_in_flight(leader: u64, term: u64): Pending -> InFlight {
                    leader: leader,
                    term: term,
                }

                mark_in_doubt(new_leader: u64, new_term: u64): InFlight -> InDoubt
                    where new_leader != self.leader || new_term != self.term
                {
                    prev_leader: self.leader,
                    prev_term: self.term,
                }

                ack(): InFlight -> Acked;
                nack(): InFlight -> Pending;
            }
        }
    }
}

#[test]
fn test_map_insert_and_contains() {
    let store = MessageStore::new();

    assert!(!store.messages_contains(&1));
    assert!(store.messages_insert(1));
    assert!(store.messages_contains(&1));

    // Duplicate insert fails
    assert!(!store.messages_insert(1));
}

#[test]
fn test_map_handle_access() {
    let store = MessageStore::new();
    store.messages_insert(1);

    // Get handle to pending item
    {
        let handle = store.messages_pending(&1).unwrap();
        assert_eq!(*handle.key(), 1);
        // Handle dropped without transition - item goes back
    }

    // Item should still be there
    assert!(store.messages_pending(&1).is_some());
}

#[test]
fn test_map_handle_transition() {
    let store = MessageStore::new();
    store.messages_insert(1);

    // Get handle and transition
    {
        let handle = store.messages_pending(&1).unwrap();
        handle.mark_in_flight(42, 100).unwrap();
    }

    // Now in in_flight, not pending
    assert!(store.messages_pending(&1).is_none());
    assert!(store.messages_in_flight(&1).is_some());

    // Check the data
    {
        let handle = store.messages_in_flight(&1).unwrap();
        assert_eq!(*handle.leader(), 42);
        assert_eq!(*handle.term(), 100);
    }
}

#[test]
fn test_map_guard_prevents_transition() {
    let store = MessageStore::new();
    store.messages_insert(1);

    // Transition to in_flight
    store.messages_pending(&1).unwrap().mark_in_flight(42, 100).unwrap();

    // Same leader/term should fail guard
    {
        let handle = store.messages_in_flight(&1).unwrap();
        let result = handle.mark_in_doubt(42, 100);
        assert!(matches!(result, Err(MessageStoreError::GuardFailed)));
    }

    // Item should still be in InFlight (handle drop put it back)
    assert_eq!(store.messages_in_flight_len(), 1);

    // Different leader should pass
    {
        let handle = store.messages_in_flight(&1).unwrap();
        handle.mark_in_doubt(99, 200).unwrap();
    }

    // Now in InDoubt
    assert_eq!(store.messages_in_flight_len(), 0);
    assert_eq!(store.messages_in_doubt_len(), 1);

    {
        let handle = store.messages_in_doubt(&1).unwrap();
        assert_eq!(*handle.prev_leader(), 42);
        assert_eq!(*handle.prev_term(), 100);
    }
}

#[test]
fn test_map_chained_transitions() {
    let store = MessageStore::new();
    store.messages_insert(1);

    // Pending -> InFlight -> Acked
    store.messages_pending(&1).unwrap().mark_in_flight(1, 100).unwrap();
    store.messages_in_flight(&1).unwrap().ack().unwrap();

    assert_eq!(store.messages_acked_len(), 1);
}

#[test]
fn test_map_nack_returns_to_pending() {
    let store = MessageStore::new();
    store.messages_insert(1);

    store.messages_pending(&1).unwrap().mark_in_flight(1, 10).unwrap();
    store.messages_in_flight(&1).unwrap().nack().unwrap();

    assert_eq!(store.messages_pending_len(), 1);
    assert_eq!(store.messages_in_flight_len(), 0);
}

#[test]
fn test_map_remove() {
    let store = MessageStore::new();
    store.messages_insert(1);
    store.messages_pending(&1).unwrap().mark_in_flight(1, 10).unwrap();

    assert!(store.messages_remove(&1));
    assert!(!store.messages_contains(&1));
    assert_eq!(store.messages_len(), 0);
}

#[test]
fn test_map_keys_iteration() {
    let store = MessageStore::new();

    for i in 0..5 {
        store.messages_insert(i);
    }

    // Move some to in_flight
    store.messages_pending(&1).unwrap().mark_in_flight(1, 10).unwrap();
    store.messages_pending(&3).unwrap().mark_in_flight(1, 30).unwrap();

    let pending_keys = store.messages_pending_keys();
    assert_eq!(pending_keys.len(), 3);

    let in_flight_keys = store.messages_in_flight_keys();
    assert_eq!(in_flight_keys.len(), 2);
}

// ============================================================================
// Queue-based store tests
// ============================================================================

store! {
    pub TaskStore {
        tasks: Queue {
            key id: u64,

            states {
                Pending,
                Running { worker: u64 },
                Complete { result: i32 },
            }

            transitions {
                start(worker: u64): Pending -> Running {
                    worker: worker,
                }

                complete(result: i32): Running -> Complete {
                    result: result,
                }

                fail(): Running -> Pending;
            }
        }
    }
}

#[test]
fn test_queue_push() {
    let store = TaskStore::new();

    let id1 = store.tasks_push();
    let id2 = store.tasks_push();
    let id3 = store.tasks_push();

    assert_eq!(id1, 0);
    assert_eq!(id2, 1);
    assert_eq!(id3, 2);
    assert_eq!(store.tasks_len(), 3);
    assert_eq!(store.tasks_pending_len(), 3);
}

#[test]
fn test_queue_fifo_order() {
    let store = TaskStore::new();

    store.tasks_push();
    store.tasks_push();
    store.tasks_push();

    // Pop in FIFO order
    {
        let handle = store.tasks_next_pending().unwrap();
        assert_eq!(*handle.id(), 0);
        handle.start(100).unwrap();
    }

    {
        let handle = store.tasks_next_pending().unwrap();
        assert_eq!(*handle.id(), 1);
        handle.start(101).unwrap();
    }

    {
        let handle = store.tasks_next_pending().unwrap();
        assert_eq!(*handle.id(), 2);
        // Drop without transition - goes back to front
    }

    // Item 2 should be back at front
    {
        let handle = store.tasks_next_pending().unwrap();
        assert_eq!(*handle.id(), 2);
    }
}

#[test]
fn test_queue_handle_carries_id() {
    let store = TaskStore::new();

    let original_id = store.tasks_push();

    // Transition through states - id is preserved
    store.tasks_next_pending().unwrap().start(42).unwrap();

    {
        let handle = store.tasks_next_running().unwrap();
        assert_eq!(*handle.id(), original_id);
        assert_eq!(*handle.worker(), 42);
        handle.complete(0).unwrap();
    }

    let peeked = store.tasks_peek_complete().unwrap();
    assert_eq!(peeked.id, original_id);
    assert_eq!(peeked.result, 0);
}

#[test]
fn test_queue_fail_returns_to_pending() {
    let store = TaskStore::new();

    store.tasks_push();
    store.tasks_next_pending().unwrap().start(1).unwrap();

    assert_eq!(store.tasks_pending_len(), 0);
    assert_eq!(store.tasks_running_len(), 1);

    store.tasks_next_running().unwrap().fail().unwrap();

    assert_eq!(store.tasks_pending_len(), 1);
    assert_eq!(store.tasks_running_len(), 0);
}

#[test]
fn test_queue_peek() {
    let store = TaskStore::new();

    store.tasks_push();
    store.tasks_push();

    // Peek doesn't remove (returns clone)
    let peeked = store.tasks_peek_pending().unwrap();
    assert_eq!(peeked.id, 0);
    assert_eq!(store.tasks_pending_len(), 2);

    // Can peek again
    let peeked = store.tasks_peek_pending().unwrap();
    assert_eq!(peeked.id, 0);
}

#[test]
fn test_queue_multiple_in_running() {
    let store = TaskStore::new();

    for _ in 0..5 {
        store.tasks_push();
    }

    // Start first 3
    for i in 0..3 {
        store.tasks_next_pending().unwrap().start(i).unwrap();
    }

    assert_eq!(store.tasks_pending_len(), 2);
    assert_eq!(store.tasks_running_len(), 3);

    // Complete first running (FIFO)
    {
        let handle = store.tasks_next_running().unwrap();
        assert_eq!(*handle.id(), 0);
        handle.complete(42).unwrap();
    }

    assert_eq!(store.tasks_running_len(), 2);
    assert_eq!(store.tasks_complete_len(), 1);
}

#[test]
fn test_queue_drain_complete() {
    let store = TaskStore::new();

    for _ in 0..3 {
        store.tasks_push();
    }

    // Run all to completion
    for i in 0..3 {
        store.tasks_next_pending().unwrap().start(i).unwrap();
    }
    for i in 0..3 {
        store.tasks_next_running().unwrap().complete(i as i32).unwrap();
    }

    assert_eq!(store.tasks_complete_len(), 3);

    // Peek and verify
    let first = store.tasks_peek_complete().unwrap();
    assert_eq!(first.id, 0);
    assert_eq!(first.result, 0);
}
