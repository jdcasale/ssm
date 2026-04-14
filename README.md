# ssm

Shared state machines for Rust - type-safe state machines with validated transitions, designed to put state management
on rails and avoid common footguns when dealing with shared state.

## Features

- **Type-safe transitions** - Invalid state transitions are compile-time or runtime errors
- **Handle-based API** - Get a typed handle, only valid transitions are available
- **Per-state collections** - Items stored in separate collections by state
- **Guards** - Conditional transitions with `where` clauses
- **FIFO queues** - Queue collections with automatic ID generation
- **Serialization** - Serde support for snapshots and persistence

## Installation

```toml
[dependencies]
ssm = "0.1"
serde = { version = "1", features = ["derive"] }
```

## Quick Start: Typed Store

The `store!` macro creates collections where each state has its own storage. Transitions move items between collections, giving you type-safe access.

```rust
use ssm::store;

store! {
    pub MessageStore {
        messages: Map<u64> {
            states {
                Pending,
                InFlight { leader: u64, term: u64 },
                Acked,
            }

            transitions {
                send(leader: u64, term: u64): Pending -> InFlight {
                    leader: leader,
                    term: term,
                }

                ack(): InFlight -> Acked;
                nack(): InFlight -> Pending;
            }
        }
    }
}

fn main() {
    let store = MessageStore::new();
    
    // Insert creates item in initial state (Pending)
    store.messages_insert(1);
    
    // Get a handle - only transitions valid for Pending are available
    let pending = store.messages_pending(&1).unwrap();
    pending.send(42, 100).unwrap();  // Moves to InFlight
    
    // Now access it as InFlight
    let in_flight = store.messages_in_flight(&1).unwrap();
    println!("Leader: {}", in_flight.leader());
    in_flight.ack().unwrap();  // Moves to Acked
    
    // Type-safe iteration
    for key in store.messages_acked_keys() {
        println!("Message {} completed", key);
    }
}
```

### Handle Safety

Handles enforce state validity:

```rust
let handle = store.messages_pending(&1).unwrap();

// If you drop without transitioning, item goes back
drop(handle);
assert!(store.messages_pending(&1).is_some());  // Still there

// Transition consumes the handle
let handle = store.messages_pending(&1).unwrap();
handle.send(1, 1).unwrap();
// handle is consumed, item moved to InFlight
```

### Guards

Conditional transitions with access to current state:

```rust
transitions {
    mark_in_doubt(new_leader: u64): InFlight -> InDoubt
        where new_leader != self.leader  // Guard condition
    {
        prev_leader: self.leader,  // Access source fields
    }
}
```

If the guard fails, the transition returns `Err(GuardFailed)` and the item stays in place.

## Queue Collections

For FIFO processing with automatic ID generation:

```rust
store! {
    pub TaskStore {
        tasks: Queue {
            key id: u64,  // Auto-generated, carried through states
            
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

fn main() {
    let store = TaskStore::new();
    
    // Push generates ID automatically
    let id1 = store.tasks_push();  // Returns 0
    let id2 = store.tasks_push();  // Returns 1
    
    // Process in FIFO order
    let task = store.tasks_next_pending().unwrap();
    assert_eq!(*task.id(), 0);  // First one pushed
    task.start(worker_id).unwrap();
    
    // ID preserved through transitions
    let running = store.tasks_next_running().unwrap();
    assert_eq!(*running.id(), 0);  // Same task
    running.complete(42).unwrap();
    
    // Peek without removing
    let done = store.tasks_peek_complete().unwrap();
    println!("Task {} finished with {}", done.id, done.result);
}
```

## Concurrency

Each collection uses `Arc<Mutex<>>` internally, so handles can be used from multiple threads without external locking:

```rust
use std::sync::Arc;

let store = Arc::new(MessageStore::new());

// Access from multiple threads
let store2 = Arc::clone(&store);
std::thread::spawn(move || {
    store2.messages_insert(1);
    store2.messages_pending(&1).unwrap().send(1, 1).unwrap();
});
```

## Generated Types

For a store with collection `messages`:

| Type | Description |
|------|-------------|
| `MessageStore` | The store struct |
| `MessageStoreError` | Error enum (`NotFound`, `GuardFailed`) |
| `MessagesPending` | State struct for Pending items |
| `MessagesInFlight` | State struct for InFlight items |
| `MessagesPendingHandle` | Handle for pending items with valid transitions |
| `MessagesInFlightHandle` | Handle for in-flight items with valid transitions |

## API Reference

### Map Collections

```rust
// Insert in initial state
store.messages_insert(key) -> bool

// Check existence (any state)
store.messages_contains(&key) -> bool

// Remove from any state
store.messages_remove(&key) -> bool

// Total count
store.messages_len() -> usize

// Per-state access (returns handle)
store.messages_pending(&key) -> Option<MessagesPendingHandle>
store.messages_in_flight(&key) -> Option<MessagesInFlightHandle>

// Per-state counts
store.messages_pending_len() -> usize
store.messages_in_flight_len() -> usize

// Per-state keys
store.messages_pending_keys() -> Vec<K>
```

### Queue Collections

```rust
// Push to initial state (returns generated ID)
store.tasks_push() -> KeyType

// Total count
store.tasks_len() -> usize

// FIFO access (returns handle)
store.tasks_next_pending() -> Option<TasksPendingHandle>
store.tasks_next_running() -> Option<TasksRunningHandle>

// Peek without removing
store.tasks_peek_pending() -> Option<&TasksPending>

// Per-state counts
store.tasks_pending_len() -> usize
store.tasks_running_len() -> usize
```

### Handles

```rust
// Access fields
handle.field_name() -> &FieldType

// For Map handles
handle.key() -> &KeyType

// For Queue handles
handle.id() -> &KeyType

// Transitions (consumes handle)
handle.transition_name(args) -> Result<(), StoreError>
```

## License

MIT
