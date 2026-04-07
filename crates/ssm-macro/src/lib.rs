use proc_macro::TokenStream;
use syn::parse_macro_input;

mod codegen;
mod parse;

/// Define a state machine with typestate pattern and actor execution.
///
/// # Example
///
/// ```ignore
/// use stactor::state_machine;
///
/// state_machine! {
///     pub enum Door {
///         #[initial]
///         Closed,
///         Open { opened_at: u64 },
///         Locked,
///     }
///
///     impl Door {
///         fn open(self: Closed) -> Open {
///             DoorStateData::Open { opened_at: 123 }
///         }
///
///         fn close(self: Open) -> Closed {
///             DoorStateData::Closed
///         }
///
///         #[guard(key == 1234)]
///         fn lock(self: Closed, key: u32) -> Locked {
///             DoorStateData::Locked
///         }
///     }
/// }
/// ```
///
/// This generates:
/// - `Door::spawn()` - spawns actor, returns `DoorHandle<Closed>`
/// - `DoorHandle<S>` - typed handle with state-specific methods
/// - `DoorStateData` - enum of all states (serializable)
/// - `handle.get_state()` / `handle.snapshot()` - state inspection
#[proc_macro]
pub fn state_machine(input: TokenStream) -> TokenStream {
    let sm = parse_macro_input!(input as parse::StateMachine);

    match codegen::generate(sm) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
