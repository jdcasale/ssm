//! Code generation for state machine syntax.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::Result;

use std::collections::HashMap;
use super::parse::{HookKind, LifecycleHook, State, StateMachine, Transition};

/// Convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Generate all code from a state machine definition.
pub fn generate(sm: StateMachine) -> Result<TokenStream2> {
    let vis = &sm.vis;
    let name = &sm.name;
    let handle_name = format_ident!("{}", name);
    let state_data_name = format_ident!("{}StateData", name);
    let snapshot_name = format_ident!("{}Snapshot", name);
    let context_name = format_ident!("{}Context", name);
    let inner_name = format_ident!("__{}Inner", name);

    // State data enum
    let state_variants = generate_state_variants(&sm.states);

    // Context type - use provided type or generate empty struct
    let (context_def, context_type, context_needs_serde) = if let Some(ctx) = &sm.context {
        let ty = &ctx.ty;
        (quote! {}, quote! { #ty }, false)
    } else {
        (quote! {
            #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
            pub struct #context_name;
        }, quote! { #context_name }, true)
    };

    // Error types
    let error_types = generate_error_types(&sm.transitions);

    // Build state lookup
    let state_map: HashMap<String, &State> = sm.states.iter()
        .map(|s| (s.name.to_string(), s))
        .collect();

    // State constructors for use in transition bodies
    let state_constructors = generate_state_constructors(&state_data_name, &sm.states);

    // Lifecycle hook methods
    let hook_methods = generate_hook_methods(&state_data_name, &sm.hooks, &state_map);

    // Build hook lookup for use in transitions
    let hook_map = build_hook_map(&sm.hooks);

    // Handler methods on inner struct
    let sm_name_str = name.to_string();
    let handler_methods = generate_handler_methods(
        &state_data_name,
        &sm.transitions,
        &state_map,
        &state_constructors,
        &sm_name_str,
        &hook_map,
    );

    // Handle methods (both sync and async versions)
    let sync_handle_methods = generate_handle_methods_sync(&sm.transitions);
    let async_handle_methods = generate_handle_methods_async(&sm.transitions);

    // Initial state
    let initial = sm.states.iter().find(|s| s.is_initial).unwrap();
    let initial_name = &initial.name;
    let initial_data = if initial.fields.is_empty() {
        quote! { #state_data_name::#initial_name }
    } else {
        let defaults: Vec<_> = initial.fields.iter().map(|f| {
            let fname = &f.name;
            quote! { #fname: Default::default() }
        }).collect();
        quote! { #state_data_name::#initial_name { #(#defaults),* } }
    };

    // Snapshot serde bounds
    let snapshot_derive = if context_needs_serde {
        quote! { #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] }
    } else {
        quote! { #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] }
    };

    Ok(quote! {
        // State data enum
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #vis enum #state_data_name {
            #(#state_variants),*
        }

        // Snapshot - contains both state and context for Raft
        #snapshot_derive
        #vis struct #snapshot_name {
            pub state: #state_data_name,
            pub context: #context_type,
        }

        // Context
        #context_def

        // Error types
        #(#error_types)*

        // Inner state (sync version uses std RwLock)
        #[cfg(not(feature = "async"))]
        struct #inner_name {
            state: #state_data_name,
            ctx: #context_type,
        }

        #[cfg(not(feature = "async"))]
        impl #inner_name {
            #(#hook_methods)*
            #(#handler_methods)*
        }

        // Inner state (async version uses tokio RwLock)
        #[cfg(feature = "async")]
        struct #inner_name {
            state: #state_data_name,
            ctx: #context_type,
        }

        #[cfg(feature = "async")]
        impl #inner_name {
            #(#hook_methods)*
            #(#handler_methods)*
        }

        // Handle - sync version
        #[cfg(not(feature = "async"))]
        #[derive(Clone)]
        #vis struct #handle_name {
            inner: std::sync::Arc<std::sync::RwLock<#inner_name>>,
        }

        #[cfg(not(feature = "async"))]
        impl std::fmt::Debug for #handle_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!(#handle_name)).finish_non_exhaustive()
            }
        }

        #[cfg(not(feature = "async"))]
        impl #handle_name {
            /// Create a new state machine with default context.
            pub fn new() -> Self {
                Self::with_context(<#context_type>::default())
            }

            /// Create with a custom context.
            pub fn with_context(ctx: #context_type) -> Self {
                Self {
                    inner: std::sync::Arc::new(std::sync::RwLock::new(#inner_name {
                        state: #initial_data,
                        ctx,
                    })),
                }
            }

            /// Restore from a snapshot (e.g., after Raft snapshot recovery).
            pub fn from_snapshot(snapshot: #snapshot_name) -> Self {
                Self {
                    inner: std::sync::Arc::new(std::sync::RwLock::new(#inner_name {
                        state: snapshot.state,
                        ctx: snapshot.context,
                    })),
                }
            }

            /// Get a clone of the current state data.
            pub fn get_state(&self) -> #state_data_name {
                self.inner.read().unwrap().state.clone()
            }

            /// Get a full snapshot (state + context) for Raft persistence.
            pub fn snapshot(&self) -> #snapshot_name {
                let guard = self.inner.read().unwrap();
                #snapshot_name {
                    state: guard.state.clone(),
                    context: guard.ctx.clone(),
                }
            }

            #(#sync_handle_methods)*
        }

        #[cfg(not(feature = "async"))]
        impl Default for #handle_name {
            fn default() -> Self {
                Self::new()
            }
        }

        // Handle - async version
        #[cfg(feature = "async")]
        #[derive(Clone)]
        #vis struct #handle_name {
            inner: std::sync::Arc<tokio::sync::RwLock<#inner_name>>,
        }

        #[cfg(feature = "async")]
        impl std::fmt::Debug for #handle_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!(#handle_name)).finish_non_exhaustive()
            }
        }

        #[cfg(feature = "async")]
        impl #handle_name {
            /// Create a new state machine with default context.
            pub fn new() -> Self {
                Self::with_context(<#context_type>::default())
            }

            /// Create with a custom context.
            pub fn with_context(ctx: #context_type) -> Self {
                Self {
                    inner: std::sync::Arc::new(tokio::sync::RwLock::new(#inner_name {
                        state: #initial_data,
                        ctx,
                    })),
                }
            }

            /// Restore from a snapshot (e.g., after Raft snapshot recovery).
            pub fn from_snapshot(snapshot: #snapshot_name) -> Self {
                Self {
                    inner: std::sync::Arc::new(tokio::sync::RwLock::new(#inner_name {
                        state: snapshot.state,
                        ctx: snapshot.context,
                    })),
                }
            }

            /// Get a clone of the current state data.
            pub async fn get_state(&self) -> #state_data_name {
                self.inner.read().await.state.clone()
            }

            /// Get a full snapshot (state + context) for Raft persistence.
            pub async fn snapshot(&self) -> #snapshot_name {
                let guard = self.inner.read().await;
                #snapshot_name {
                    state: guard.state.clone(),
                    context: guard.ctx.clone(),
                }
            }

            #(#async_handle_methods)*
        }

        #[cfg(feature = "async")]
        impl Default for #handle_name {
            fn default() -> Self {
                Self::new()
            }
        }
    })
}

fn generate_state_variants(states: &[State]) -> Vec<TokenStream2> {
    states.iter().map(|s| {
        let name = &s.name;
        if s.fields.is_empty() {
            quote! { #name }
        } else {
            let fields: Vec<_> = s.fields.iter().map(|f| {
                let fname = &f.name;
                let fty = &f.ty;
                quote! { #fname: #fty }
            }).collect();
            quote! { #name { #(#fields),* } }
        }
    }).collect()
}

/// Generate constructor functions for state variants.
fn generate_state_constructors(state_data_name: &syn::Ident, states: &[State]) -> Vec<TokenStream2> {
    states.iter().map(|s| {
        let name = &s.name;
        if s.fields.is_empty() {
            quote! {
                #[allow(non_upper_case_globals)]
                const #name: #state_data_name = #state_data_name::#name;
            }
        } else {
            let field_names: Vec<_> = s.fields.iter().map(|f| &f.name).collect();
            let field_types: Vec<_> = s.fields.iter().map(|f| &f.ty).collect();
            quote! {
                #[allow(non_snake_case)]
                fn #name(#(#field_names: #field_types),*) -> #state_data_name {
                    #state_data_name::#name { #(#field_names),* }
                }
            }
        }
    }).collect()
}

fn generate_error_types(transitions: &[Transition]) -> Vec<TokenStream2> {
    transitions.iter().filter_map(|t| {
        // Generate error type if no user-provided error type
        if t.error_type.is_none() {
            let cmd_name = format_ident!("{}", to_pascal_case(&t.name.to_string()));
            let err_name = format_ident!("{}Error", cmd_name);

            let variants = if t.guard.is_some() {
                quote! {
                    InvalidState,
                    GuardFailed,
                }
            } else {
                quote! {
                    InvalidState,
                }
            };

            let display_arms = if t.guard.is_some() {
                quote! {
                    Self::InvalidState => write!(f, "invalid state for this transition"),
                    Self::GuardFailed => write!(f, "guard condition failed"),
                }
            } else {
                quote! {
                    Self::InvalidState => write!(f, "invalid state for this transition"),
                }
            };

            Some(quote! {
                #[derive(Debug, Clone, PartialEq)]
                pub enum #err_name {
                    #variants
                }

                impl std::fmt::Display for #err_name {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        match self {
                            #display_arms
                        }
                    }
                }

                impl std::error::Error for #err_name {}
            })
        } else {
            None
        }
    }).collect()
}

fn generate_handle_methods_sync(transitions: &[Transition]) -> Vec<TokenStream2> {
    transitions.iter().map(|t| {
        let method_name = &t.name;
        let handler_name = format_ident!("handle_{}", t.name);
        let cmd_name = format_ident!("{}", to_pascal_case(&t.name.to_string()));
        let err_name = format_ident!("{}Error", cmd_name);
        let arg_names: Vec<_> = t.args.iter().map(|(n, _)| n).collect();
        let arg_decls: Vec<_> = t.args.iter().map(|(n, ty)| quote! { #n: #ty }).collect();

        if let Some(err_ty) = &t.error_type {
            quote! {
                pub fn #method_name(&self, #(#arg_decls),*) -> Result<(), ssm::TransitionError<#err_ty>> {
                    let mut guard = self.inner.write().unwrap();
                    match guard.#handler_name(#(#arg_names),*) {
                        Ok(()) => Ok(()),
                        Err(None) => Err(ssm::TransitionError::InvalidState),
                        Err(Some(e)) => Err(ssm::TransitionError::User(e)),
                    }
                }
            }
        } else {
            quote! {
                pub fn #method_name(&self, #(#arg_decls),*) -> Result<(), #err_name> {
                    let mut guard = self.inner.write().unwrap();
                    guard.#handler_name(#(#arg_names),*)
                }
            }
        }
    }).collect()
}

fn generate_handle_methods_async(transitions: &[Transition]) -> Vec<TokenStream2> {
    transitions.iter().map(|t| {
        let method_name = &t.name;
        let handler_name = format_ident!("handle_{}", t.name);
        let cmd_name = format_ident!("{}", to_pascal_case(&t.name.to_string()));
        let err_name = format_ident!("{}Error", cmd_name);
        let arg_names: Vec<_> = t.args.iter().map(|(n, _)| n).collect();
        let arg_decls: Vec<_> = t.args.iter().map(|(n, ty)| quote! { #n: #ty }).collect();

        if let Some(err_ty) = &t.error_type {
            quote! {
                pub async fn #method_name(&self, #(#arg_decls),*) -> Result<(), ssm::TransitionError<#err_ty>> {
                    let mut guard = self.inner.write().await;
                    match guard.#handler_name(#(#arg_names),*) {
                        Ok(()) => Ok(()),
                        Err(None) => Err(ssm::TransitionError::InvalidState),
                        Err(Some(e)) => Err(ssm::TransitionError::User(e)),
                    }
                }
            }
        } else {
            quote! {
                pub async fn #method_name(&self, #(#arg_decls),*) -> Result<(), #err_name> {
                    let mut guard = self.inner.write().await;
                    guard.#handler_name(#(#arg_names),*)
                }
            }
        }
    }).collect()
}

/// Generate handler methods on the inner struct.
fn generate_handler_methods(
    state_data_name: &syn::Ident,
    transitions: &[Transition],
    state_map: &HashMap<String, &State>,
    state_constructors: &[TokenStream2],
    sm_name: &str,
    hook_map: &HookMap,
) -> Vec<TokenStream2> {
    transitions.iter().map(|t| {
        let cmd_name = format_ident!("{}", to_pascal_case(&t.name.to_string()));
        let err_name = format_ident!("{}Error", cmd_name);
        let handler_name = format_ident!("handle_{}", t.name);
        let transition_name = t.name.to_string();
        let arg_decls: Vec<_> = t.args.iter().map(|(n, ty)| quote! { #n: #ty }).collect();
        let body = &t.body;
        let target_state = &t.target_state;

        // Generate on_exit hook calls for source states
        let on_exit_calls: Vec<_> = t.source_states.iter().filter_map(|source| {
            let state_str = source.to_string();
            if let Some((_, Some(exit_hook))) = hook_map.get(&state_str) {
                Some(quote! {
                    #state_data_name::#source { .. } => self.#exit_hook(),
                })
            } else {
                None
            }
        }).collect();

        let on_exit_call = if on_exit_calls.is_empty() {
            quote! {}
        } else {
            quote! {
                match &self.state {
                    #(#on_exit_calls)*
                    _ => {}
                }
            }
        };

        // Generate on_enter hook call for target state
        let on_enter_call = if let Some((Some(enter_hook), _)) = hook_map.get(&target_state.to_string()) {
            quote! { self.#enter_hook(); }
        } else {
            quote! {}
        };

        // Extract fields as local variables for single-source
        let field_extractions: Vec<_> = t.source_states.iter().filter_map(|source| {
            let state = state_map.get(&source.to_string())?;
            if state.fields.is_empty() {
                return None;
            }
            let field_names: Vec<_> = state.fields.iter().map(|f| &f.name).collect();
            let field_types: Vec<_> = state.fields.iter().map(|f| &f.ty).collect();
            Some(quote! {
                #[allow(unused_variables)]
                let (#(#field_names,)*): (#(#field_types,)*) = match &self.state {
                    #state_data_name::#source { #(#field_names,)* } => (#(#field_names.clone(),)*),
                    _ => unreachable!(),
                };
            })
        }).collect();

        let field_extraction = if t.source_states.len() == 1 {
            quote! { #(#field_extractions)* }
        } else {
            quote! {
                #[allow(unused_variables)]
                let state = &self.state;
            }
        };

        // Source state check
        let source_patterns: Vec<_> = t.source_states.iter().map(|s| {
            quote! { #state_data_name::#s { .. } }
        }).collect();

        // Generate handler based on error handling mode
        if let Some(err_ty) = &t.error_type {
            // User error type - returns Result<(), Option<E>>
            let guard_check = if let Some(guard) = &t.guard {
                quote! {
                    if !(#guard) {
                        return Err(None);
                    }
                }
            } else {
                quote! {}
            };

            quote! {
                #[cfg_attr(
                    feature = "ssm-metrics",
                    metrics_helper_macros::instrument(
                        counter = "ssm_transitions_total",
                        histogram = "ssm_transition_duration_seconds",
                        error_counter = "ssm_transition_errors_total",
                        labels(state_machine = #sm_name, transition = #transition_name),
                    )
                )]
                fn #handler_name(&mut self, #(#arg_decls),*) -> Result<(), Option<#err_ty>> {
                    if !matches!(self.state, #(#source_patterns)|*) {
                        return Err(None);
                    }

                    #field_extraction

                    #[allow(unused_variables)]
                    let ctx = &mut self.ctx;

                    #guard_check

                    #(#state_constructors)*
                    match { #body } {
                        Ok(new_state) => {
                            #on_exit_call
                            self.state = new_state;
                            #on_enter_call
                            Ok(())
                        }
                        Err(e) => Err(Some(e)),
                    }
                }
            }
        } else if t.guard.is_some() {
            // Generated error type with guard
            let guard = t.guard.as_ref().unwrap();

            quote! {
                #[cfg_attr(
                    feature = "ssm-metrics",
                    metrics_helper_macros::instrument(
                        counter = "ssm_transitions_total",
                        histogram = "ssm_transition_duration_seconds",
                        error_counter = "ssm_transition_errors_total",
                        labels(state_machine = #sm_name, transition = #transition_name),
                    )
                )]
                fn #handler_name(&mut self, #(#arg_decls),*) -> Result<(), #err_name> {
                    if !matches!(self.state, #(#source_patterns)|*) {
                        return Err(#err_name::InvalidState);
                    }

                    #field_extraction

                    #[allow(unused_variables)]
                    let ctx = &mut self.ctx;

                    if !(#guard) {
                        return Err(#err_name::GuardFailed);
                    }

                    #(#state_constructors)*
                    #on_exit_call
                    self.state = { #body };
                    #on_enter_call
                    Ok(())
                }
            }
        } else {
            // Generated error type, no guard
            quote! {
                #[cfg_attr(
                    feature = "ssm-metrics",
                    metrics_helper_macros::instrument(
                        counter = "ssm_transitions_total",
                        histogram = "ssm_transition_duration_seconds",
                        error_counter = "ssm_transition_errors_total",
                        labels(state_machine = #sm_name, transition = #transition_name),
                    )
                )]
                fn #handler_name(&mut self, #(#arg_decls),*) -> Result<(), #err_name> {
                    if !matches!(self.state, #(#source_patterns)|*) {
                        return Err(#err_name::InvalidState);
                    }

                    #field_extraction

                    #[allow(unused_variables)]
                    let ctx = &mut self.ctx;

                    #(#state_constructors)*
                    #on_exit_call
                    self.state = { #body };
                    #on_enter_call
                    Ok(())
                }
            }
        }
    }).collect()
}

/// Maps state name -> (on_enter hook name, on_exit hook name)
type HookMap = HashMap<String, (Option<syn::Ident>, Option<syn::Ident>)>;

fn build_hook_map(hooks: &[LifecycleHook]) -> HookMap {
    let mut map: HookMap = HashMap::new();

    for hook in hooks {
        let state_name = hook.state.to_string();
        let entry = map.entry(state_name).or_insert((None, None));
        match hook.kind {
            HookKind::OnEnter => entry.0 = Some(hook.name.clone()),
            HookKind::OnExit => entry.1 = Some(hook.name.clone()),
        }
    }

    map
}

fn generate_hook_methods(
    state_data_name: &syn::Ident,
    hooks: &[LifecycleHook],
    state_map: &HashMap<String, &State>,
) -> Vec<TokenStream2> {
    hooks.iter().map(|hook| {
        let name = &hook.name;
        let body = &hook.body;
        let state_name = &hook.state;

        // Extract fields as local variables if the state has fields
        let field_extraction = if let Some(state) = state_map.get(&state_name.to_string()) {
            if !state.fields.is_empty() {
                let field_names: Vec<_> = state.fields.iter().map(|f| &f.name).collect();
                let field_types: Vec<_> = state.fields.iter().map(|f| &f.ty).collect();
                quote! {
                    #[allow(unused_variables)]
                    let (#(#field_names,)*): (#(#field_types,)*) = match &self.state {
                        #state_data_name::#state_name { #(#field_names,)* } => (#(#field_names.clone(),)*),
                        _ => return, // Not in expected state, skip hook
                    };
                }
            } else {
                quote! {}
            }
        } else {
            quote! {}
        };

        quote! {
            #[allow(dead_code)]
            fn #name(&mut self) {
                #field_extraction

                #[allow(unused_variables)]
                let ctx = &mut self.ctx;

                #body
            }
        }
    }).collect()
}
