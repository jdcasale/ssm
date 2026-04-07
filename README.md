# ssm

Shared state machines for Rust - thread-safe state machines with RwLock, designed for Raft and distributed systems.

## Features

- **Thread-safe** - `Arc<RwLock<...>>` under the hood, clone handles freely
- **Sync or async** - Choose `std::sync::RwLock` or `tokio::sync::RwLock`
- **Runtime validation** - Invalid transitions return errors, never panic
- **State inspection** - Query current state via read lock (concurrent reads)
- **Snapshots** - Serialize full state (data + context) for Raft snapshots
- **Restore** - Create from a snapshot to recover after restart
- **Guards** - Conditional transitions with `#[guard(...)]`
- **User error types** - Return `Result<State, YourError>` from transitions
- **Metrics** - Optional instrumentation via `ssm-metrics` feature

## Installation

```toml
[dependencies]
ssm = "0.1"
serde = { version = "1", features = ["derive"] }
```

For async contexts (e.g., tokio-based applications), enable the `async` feature:

```toml
[dependencies]
ssm = { version = "0.1", features = ["async"] }
```

This switches from `std::sync::RwLock` to `tokio::sync::RwLock` and makes all methods async.

## Quick Start

```rust
use ssm::state_machine;

state_machine! {
    pub enum Door {
        #[initial]
        Closed,
        Open { opened_at: u64 },
        Locked,
    }

    impl Door {
        fn open(self: Closed) -> Open {
            DoorStateData::Open { opened_at: 123 }
        }

        fn close(self: Open) -> Closed {
            DoorStateData::Closed
        }

        #[guard(key == 1234)]
        fn lock(self: Closed, key: u32) -> Locked {
            DoorStateData::Locked
        }
    }
}

fn main() {
    let door = Door::new();
    
    // Transitions are synchronous (or async with feature)
    door.open().unwrap();
    door.close().unwrap();
    
    // Invalid transitions return errors (don't panic)
    let err = door.open().unwrap_err();
    assert_eq!(err, OpenError::InvalidState);
    
    // Guards
    let err = door.lock(9999).unwrap_err();
    assert_eq!(err, LockError::GuardFailed);
    
    door.lock(1234).unwrap(); // Correct key
    
    // State inspection (read lock, concurrent safe)
    let state = door.get_state();
    println!("{:?}", state); // DoorStateData::Locked
    
    // Clone handles for concurrent access
    let door2 = door.clone();
    // Both door and door2 talk to the same state machine
}
```

## Context (Mutable State)

For state that persists across transitions but isn't part of the state enum:

```rust
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BankContext {
    balance: i64,
}

state_machine! {
    pub enum BankAccount {
        #[initial]
        Idle,
        Processing,
    }

    type Context = BankContext;

    impl BankAccount {
        fn begin(self: Idle) -> Processing {
            BankAccountStateData::Processing
        }

        fn deposit(self: Processing, amount: i64) -> Idle {
            ctx.balance += amount;  // ctx is available in transition bodies
            BankAccountStateData::Idle
        }
    }
}
```

## User Error Types

Return `Result<State, YourError>` for application-level errors:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum BankError {
    InsufficientFunds,
    InvalidAmount,
}

state_machine! {
    pub enum Account {
        #[initial]
        Active,
    }

    type Context = BankContext;

    impl Account {
        fn withdraw(self: Active, amount: i64) -> Result<Active, BankError> {
            if amount <= 0 {
                Err(BankError::InvalidAmount)
            } else if ctx.balance < amount {
                Err(BankError::InsufficientFunds)
            } else {
                ctx.balance -= amount;
                Ok(AccountStateData::Active)
            }
        }
    }
}

// Handle methods return TransitionError<E> which wraps your error:
// - TransitionError::User(e) - your error
// - TransitionError::InvalidState - wrong source state
```

## Multi-Source Transitions

Transitions from multiple states:

```rust
state_machine! {
    pub enum Msg {
        #[initial]
        Pending,
        InFlight { leader: u64 },
        InDoubt { previous_leader: u64 },
        Done,
    }

    impl Msg {
        // Can be called from InFlight OR InDoubt
        fn ack(self: (InFlight, InDoubt)) -> Done {
            MsgStateData::Done
        }
    }
}
```

## Snapshots and Restore

For Raft log compaction:

```rust
let counter = Counter::new();
counter.start().unwrap();
counter.increment().unwrap();

// Take snapshot (includes state + context)
let snapshot = counter.snapshot();
let json = serde_json::to_string(&snapshot).unwrap();

// Later: restore from snapshot
let restored = Counter::from_snapshot(snapshot);
// Continues from where it left off
```

## Metrics (Optional)

Enable instrumentation for debugging:

```toml
[features]
ssm-metrics = ["metrics", "metrics-helper-macros"]

[dependencies]
metrics = { version = "0.24", optional = true }
metrics-helper-macros = { version = "0.1", optional = true }
```

Emits per-transition:
- `ssm_transitions_total{state_machine, transition}` - counter
- `ssm_transition_duration_seconds{...}` - histogram  
- `ssm_transition_errors_total{...}` - error counter

Zero overhead when feature is disabled.

## Generated Types

For a state machine named `Foo`, the macro generates:

| Type | Description |
|------|-------------|
| `Foo` | Handle with `new()`, `from_snapshot()`, and transition methods |
| `FooStateData` | Enum of states with their data |
| `FooSnapshot` | State + context for serialization |
| `FooContext` | Context type (generated if not specified) |
| `{Transition}Error` | Error enum per transition |

## Implementation

Under the hood, ssm uses `Arc<RwLock<Inner>>`:
- **Reads** (`get_state()`, `snapshot()`) take a read lock - concurrent reads allowed
- **Writes** (transitions) take a write lock - serialized, exclusive access
- **Clone** is cheap - just clones the `Arc`

| Feature | Lock Type | API |
|---------|-----------|-----|
| (default) | `std::sync::RwLock` | Sync |
| `async` | `tokio::sync::RwLock` | Async |

This gives you predictable, linearizable behavior. Use `async` feature in tokio contexts to avoid blocking executor threads.

## License

MIT
