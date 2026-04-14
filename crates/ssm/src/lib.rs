//! Type-safe state machines for Rust.
//!
//! SSM provides the `store!` macro for defining collections with per-state storage
//! and handle-based transitions. Items are stored in separate collections by state,
//! and transitions move items between them with type-safe handles.
//!
//! # Example
//!
//! ```ignore
//! use ssm::store;
//!
//! store! {
//!     pub MessageStore {
//!         messages: Map<u64> {
//!             states {
//!                 Pending,
//!                 InFlight { leader: u64, term: u64 },
//!                 Acked,
//!             }
//!             transitions {
//!                 send(leader: u64, term: u64): Pending -> InFlight {
//!                     leader: leader,
//!                     term: term,
//!                 }
//!                 ack(): InFlight -> Acked;
//!             }
//!         }
//!     }
//! }
//!
//! let store = MessageStore::new();
//! store.messages_insert(1);
//!
//! let handle = store.messages_pending(&1).unwrap();
//! handle.send(42, 100).unwrap();
//!
//! let handle = store.messages_in_flight(&1).unwrap();
//! handle.ack().unwrap();
//! ```

pub use ssm_macro::store;
