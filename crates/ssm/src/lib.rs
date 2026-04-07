//! Shared state machines for Rust.
//!
//! SSM provides a macro for defining thread-safe state machines that:
//! - Use RwLock for safe concurrent access
//! - Validate transitions at runtime
//! - Support state inspection and serialization for Raft snapshots
//!
//! # Example
//!
//! ```ignore
//! use ssm::state_machine;
//!
//! state_machine! {
//!     pub enum Door {
//!         #[initial]
//!         Closed,
//!         Open { opened_at: u64 },
//!         Locked,
//!     }
//!
//!     impl Door {
//!         fn open(self: Closed) -> Open {
//!             DoorStateData::Open { opened_at: 123 }
//!         }
//!
//!         fn close(self: Open) -> Closed {
//!             DoorStateData::Closed
//!         }
//!
//!         #[guard(key == 1234)]
//!         fn lock(self: Closed, key: u32) -> Locked {
//!             DoorStateData::Locked
//!         }
//!     }
//! }
//!
//! fn main() {
//!     let door = Door::new();
//!     door.open().unwrap();
//!     door.close().unwrap();
//!
//!     // State inspection
//!     let state = door.get_state();
//!     println!("{:?}", state);
//!
//!     // Full snapshot for Raft
//!     let snapshot = door.snapshot();
//!     let json = serde_json::to_string(&snapshot).unwrap();
//!
//!     // Restore from snapshot
//!     let restored = Door::from_snapshot(snapshot);
//! }
//! ```

pub use ssm_macro::state_machine;

/// Error wrapper for transitions with user-provided error types.
///
/// When a transition defines a custom error type (e.g., `-> Result<State, MyError>`),
/// this wrapper distinguishes between user errors and invalid state errors.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionError<E> {
    /// The transition failed with a user-defined error.
    User(E),
    /// The state machine was in an invalid state for this transition.
    InvalidState,
}

impl<E: std::fmt::Display> std::fmt::Display for TransitionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(e) => write!(f, "{}", e),
            Self::InvalidState => write!(f, "invalid state for this transition"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for TransitionError<E> {}

impl<E> TransitionError<E> {
    /// Returns the user error if this is a `User` variant.
    pub fn user_error(self) -> Option<E> {
        match self {
            Self::User(e) => Some(e),
            _ => None,
        }
    }

    /// Returns true if this was an invalid state transition.
    pub fn is_invalid_state(&self) -> bool {
        matches!(self, Self::InvalidState)
    }
}
