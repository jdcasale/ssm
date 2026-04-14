use proc_macro::TokenStream;
use syn::parse_macro_input;

mod typed_store_codegen;
mod typed_store_parse;

/// Typed store with per-state collections and handle-based transitions.
///
/// Each state gets its own collection (HashMap for Map, VecDeque for Queue),
/// and transitions move items between them. Handles provide type-safe access
/// to items and only expose valid transitions for that state.
///
/// # Map Example
///
/// ```ignore
/// store! {
///     pub MessageStore {
///         messages: Map<u64> {
///             states {
///                 Pending,
///                 InFlight { leader: u64, term: u64 },
///                 Acked,
///             }
///             transitions {
///                 send(leader: u64, term: u64): Pending -> InFlight {
///                     leader: leader,
///                     term: term,
///                 }
///                 ack(): InFlight -> Acked;
///             }
///         }
///     }
/// }
///
/// let store = MessageStore::new();
/// store.messages_insert(1);
///
/// // Get handle - only valid transitions available
/// let handle = store.messages_pending(&1).unwrap();
/// handle.send(42, 100).unwrap();  // Moves to InFlight
/// ```
///
/// # Queue Example
///
/// ```ignore
/// store! {
///     pub TaskStore {
///         tasks: Queue {
///             key id: u64,  // Auto-generated, carried through states
///             states {
///                 Pending,
///                 Running { worker: u64 },
///                 Complete,
///             }
///             transitions {
///                 start(worker: u64): Pending -> Running { worker: worker }
///                 complete(): Running -> Complete;
///             }
///         }
///     }
/// }
///
/// let store = TaskStore::new();
/// let id = store.tasks_push();  // Returns generated ID
///
/// // FIFO processing
/// let task = store.tasks_next_pending().unwrap();
/// task.start(42).unwrap();
/// ```
#[proc_macro]
pub fn store(input: TokenStream) -> TokenStream {
    let store = parse_macro_input!(input as typed_store_parse::TypedStore);

    match typed_store_codegen::generate(store) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
