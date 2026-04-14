//! Code generation for typed store! macro.
//!
//! Generates per-state collections with handle-based typestate transitions.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use std::collections::HashMap;
use syn::Result;

use super::typed_store_parse::{
    CollectionKind, FieldAssignment, TypedCollection, TypedState, TypedStore, TypedTransition,
};

pub fn generate(store: TypedStore) -> Result<TokenStream2> {
    let vis = &store.vis;
    let name = &store.name;
    let error_name = format_ident!("{}Error", name);

    let mut all_items = Vec::new();

    for collection in &store.collections {
        let items = generate_collection(collection, name, &error_name, vis)?;
        all_items.push(items);
    }

    // Error type
    let error_type = quote! {
        #[derive(Debug, Clone, PartialEq)]
        #vis enum #error_name {
            /// Item not found in expected state.
            NotFound,
            /// Guard condition failed.
            GuardFailed,
        }

        impl std::fmt::Display for #error_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    Self::NotFound => write!(f, "item not found in expected state"),
                    Self::GuardFailed => write!(f, "guard condition failed"),
                }
            }
        }

        impl std::error::Error for #error_name {}
    };

    Ok(quote! {
        #error_type
        #(#all_items)*
    })
}

/// Get the prefixed state name for a collection's state.
fn prefixed_state_name(col_prefix: &str, state_name: &syn::Ident) -> syn::Ident {
    format_ident!("{}{}", col_prefix, state_name)
}

fn generate_collection(
    collection: &TypedCollection,
    store_name: &syn::Ident,
    error_name: &syn::Ident,
    vis: &syn::Visibility,
) -> Result<TokenStream2> {
    match &collection.kind {
        CollectionKind::Map(key_type) => {
            generate_map_collection(collection, key_type, store_name, error_name, vis)
        }
        CollectionKind::Queue { key_name, key_type } => {
            generate_queue_collection(collection, key_name, key_type, store_name, error_name, vis)
        }
    }
}

fn generate_map_collection(
    collection: &TypedCollection,
    key_type: &syn::Type,
    store_name: &syn::Ident,
    error_name: &syn::Ident,
    vis: &syn::Visibility,
) -> Result<TokenStream2> {
    let col_name = &collection.name;
    let col_prefix = to_pascal_case(&col_name.to_string());

    let state_map: HashMap<String, &TypedState> = collection
        .states
        .iter()
        .map(|s| (s.name.to_string(), s))
        .collect();

    // Generate state structs with prefixed names
    let state_structs: Vec<_> = collection
        .states
        .iter()
        .map(|s| generate_state_struct_prefixed(s, &col_prefix))
        .collect();

    // Generate struct fields (one Arc<Mutex<HashMap>> per state)
    let struct_fields: Vec<_> = collection
        .states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            let state_type = prefixed_state_name(&col_prefix, &s.name);
            quote! { #field_name: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<#key_type, #state_type>>> }
        })
        .collect();

    // Generate field initializers
    let field_inits: Vec<_> = collection
        .states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! { #field_name: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())) }
        })
        .collect();

    // Generate handle structs and impls for each state
    let handles: Vec<_> = collection
        .states
        .iter()
        .map(|state| {
            generate_map_handle(
                col_name,
                &col_prefix,
                key_type,
                state,
                &collection.transitions,
                &state_map,
                store_name,
                error_name,
            )
        })
        .collect();

    // Generate store methods
    let store_methods =
        generate_map_store_methods(col_name, &col_prefix, key_type, &collection.states, &collection.transitions);

    Ok(quote! {
        // State structs
        #(#state_structs)*

        // Handle types
        #(#handles)*

        // Store struct
        #[derive(Debug, Clone, Default)]
        #vis struct #store_name {
            #(#struct_fields),*
        }

        impl #store_name {
            /// Create a new empty store.
            pub fn new() -> Self {
                Self {
                    #(#field_inits),*
                }
            }

            #store_methods
        }
    })
}

fn generate_queue_collection(
    collection: &TypedCollection,
    key_name: &syn::Ident,
    key_type: &syn::Type,
    store_name: &syn::Ident,
    error_name: &syn::Ident,
    vis: &syn::Visibility,
) -> Result<TokenStream2> {
    let col_name = &collection.name;
    let col_prefix = to_pascal_case(&col_name.to_string());

    let state_map: HashMap<String, &TypedState> = collection
        .states
        .iter()
        .map(|s| (s.name.to_string(), s))
        .collect();

    // Generate state structs (with key field added)
    let state_structs: Vec<_> = collection
        .states
        .iter()
        .map(|s| generate_state_struct_with_key_prefixed(s, &col_prefix, key_name, key_type))
        .collect();

    // Generate struct fields (one Arc<Mutex<VecDeque>> per state + next_id)
    let mut struct_fields: Vec<_> = collection
        .states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            let state_type = prefixed_state_name(&col_prefix, &s.name);
            quote! { #field_name: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<#state_type>>> }
        })
        .collect();

    let next_id_field = format_ident!("{}_next_id", col_name);
    struct_fields.push(quote! { #next_id_field: std::sync::Arc<std::sync::Mutex<#key_type>> });

    // Generate field initializers
    let mut field_inits: Vec<_> = collection
        .states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! { #field_name: std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())) }
        })
        .collect();

    field_inits.push(quote! { #next_id_field: std::sync::Arc::new(std::sync::Mutex::new(Default::default())) });

    // Generate handle structs and impls for each state
    let handles: Vec<_> = collection
        .states
        .iter()
        .map(|state| {
            generate_queue_handle(
                &col_prefix,
                key_name,
                key_type,
                state,
                &collection.transitions,
                &state_map,
                error_name,
            )
        })
        .collect();

    // Generate store methods
    let store_methods = generate_queue_store_methods(
        col_name,
        &col_prefix,
        key_name,
        key_type,
        &collection.states,
        &collection.transitions,
    );

    Ok(quote! {
        // State structs (with key field)
        #(#state_structs)*

        // Handle types
        #(#handles)*

        // Store struct
        #[derive(Debug, Clone, Default)]
        #vis struct #store_name {
            #(#struct_fields),*
        }

        impl #store_name {
            /// Create a new empty store.
            pub fn new() -> Self {
                Self {
                    #(#field_inits),*
                }
            }

            #store_methods
        }
    })
}

fn generate_state_struct_prefixed(state: &TypedState, col_prefix: &str) -> TokenStream2 {
    let name = prefixed_state_name(col_prefix, &state.name);
    if state.fields.is_empty() {
        quote! {
            #[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
            pub struct #name;
        }
    } else {
        let field_defs: Vec<_> = state
            .fields
            .iter()
            .map(|f| {
                let fname = &f.name;
                let fty = &f.ty;
                quote! { pub #fname: #fty }
            })
            .collect();

        quote! {
            #[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
            pub struct #name {
                #(#field_defs),*
            }
        }
    }
}

fn generate_state_struct_with_key_prefixed(
    state: &TypedState,
    col_prefix: &str,
    key_name: &syn::Ident,
    key_type: &syn::Type,
) -> TokenStream2 {
    let name = prefixed_state_name(col_prefix, &state.name);
    let mut field_defs = vec![quote! { pub #key_name: #key_type }];

    for f in &state.fields {
        let fname = &f.name;
        let fty = &f.ty;
        field_defs.push(quote! { pub #fname: #fty });
    }

    quote! {
        #[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
        pub struct #name {
            #(#field_defs),*
        }
    }
}

fn generate_map_handle(
    col_name: &syn::Ident,
    col_prefix: &str,
    key_type: &syn::Type,
    state: &TypedState,
    transitions: &[TypedTransition],
    state_map: &HashMap<String, &TypedState>,
    _store_name: &syn::Ident,
    error_name: &syn::Ident,
) -> TokenStream2 {
    let state_name = &state.name;
    let state_type = prefixed_state_name(col_prefix, state_name);
    let snake_state = to_snake_case(&state_name.to_string());
    let handle_name = format_ident!("{}{}Handle", col_prefix, state_name);

    // Collect all target states for this handle's transitions
    let target_states: Vec<_> = transitions
        .iter()
        .filter(|t| t.sources.iter().any(|s| s == state_name))
        .map(|t| &t.target)
        .collect();

    // Generate fields for source collection and all target collections
    let source_field_name = format_ident!("source_{}", snake_state);
    let source_field = quote! {
        #source_field_name: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<#key_type, #state_type>>>
    };

    let mut target_fields = Vec::new();
    let mut seen_targets = std::collections::HashSet::new();
    for target in &target_states {
        let target_str = target.to_string();
        if seen_targets.insert(target_str.clone()) {
            let target_type = prefixed_state_name(col_prefix, target);
            let target_snake = to_snake_case(&target_str);
            let target_field_name = format_ident!("target_{}", target_snake);
            target_fields.push(quote! {
                #target_field_name: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<#key_type, #target_type>>>
            });
        }
    }

    // Find transitions from this state
    let transition_methods: Vec<_> = transitions
        .iter()
        .filter(|t| t.sources.iter().any(|s| s == state_name))
        .map(|t| {
            generate_map_handle_transition_arc(col_name, col_prefix, key_type, t, state_map, error_name)
        })
        .collect();

    // Generate accessor methods for state fields
    let field_accessors: Vec<_> = state
        .fields
        .iter()
        .map(|f| {
            let fname = &f.name;
            let fty = &f.ty;
            quote! {
                pub fn #fname(&self) -> &#fty {
                    &self.item.as_ref().unwrap().#fname
                }
            }
        })
        .collect();

    quote! {
        pub struct #handle_name {
            #source_field,
            #(#target_fields,)*
            key: #key_type,
            item: Option<#state_type>,
        }

        impl #handle_name {
            /// Get the key for this item.
            pub fn key(&self) -> &#key_type {
                &self.key
            }

            #(#field_accessors)*

            #(#transition_methods)*
        }

        impl Drop for #handle_name {
            fn drop(&mut self) {
                // If item wasn't consumed by a transition, put it back
                if let Some(item) = self.item.take() {
                    self.#source_field_name.lock().unwrap().insert(self.key.clone(), item);
                }
            }
        }
    }
}

fn generate_map_handle_transition_arc(
    _col_name: &syn::Ident,
    col_prefix: &str,
    _key_type: &syn::Type,
    transition: &TypedTransition,
    state_map: &HashMap<String, &TypedState>,
    error_name: &syn::Ident,
) -> TokenStream2 {
    let method_name = &transition.name;
    let target = &transition.target;
    let target_type = prefixed_state_name(col_prefix, target);
    let target_snake = to_snake_case(&target.to_string());
    let target_field = format_ident!("target_{}", target_snake);

    let param_decls: Vec<_> = transition
        .params
        .iter()
        .map(|p| {
            let pname = &p.name;
            let pty = &p.ty;
            quote! { #pname: #pty }
        })
        .collect();

    let target_state = state_map.get(&target.to_string());

    // Build target construction
    let target_construction =
        build_target_construction(&target_type, target_state, &transition.field_assignments);

    // Guard check
    let guard_check = if let Some(guard) = &transition.guard {
        let rewritten = rewrite_self_to_item(quote! { #guard });
        quote! {
            if !(#rewritten) {
                return Err(#error_name::GuardFailed);
            }
        }
    } else {
        quote! {}
    };

    quote! {
        pub fn #method_name(mut self, #(#param_decls),*) -> Result<(), #error_name> {
            let item = self.item.as_ref().unwrap();
            #guard_check
            let item = self.item.take().unwrap();
            self.#target_field.lock().unwrap().insert(self.key.clone(), #target_construction);
            Ok(())
        }
    }
}

fn generate_queue_handle(
    col_prefix: &str,
    key_name: &syn::Ident,
    key_type: &syn::Type,
    state: &TypedState,
    transitions: &[TypedTransition],
    state_map: &HashMap<String, &TypedState>,
    error_name: &syn::Ident,
) -> TokenStream2 {
    let state_name = &state.name;
    let state_type = prefixed_state_name(col_prefix, state_name);
    let snake_state = to_snake_case(&state_name.to_string());
    let handle_name = format_ident!("{}{}Handle", col_prefix, state_name);

    // Collect all target states for this handle's transitions
    let target_states: Vec<_> = transitions
        .iter()
        .filter(|t| t.sources.iter().any(|s| s == state_name))
        .map(|t| &t.target)
        .collect();

    // Generate fields for source collection and all target collections
    let source_field_name = format_ident!("source_{}", snake_state);
    let source_field = quote! {
        #source_field_name: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<#state_type>>>
    };

    let mut target_fields = Vec::new();
    let mut seen_targets = std::collections::HashSet::new();
    for target in &target_states {
        let target_str = target.to_string();
        if seen_targets.insert(target_str.clone()) {
            let target_type = prefixed_state_name(col_prefix, target);
            let target_snake = to_snake_case(&target_str);
            let target_field_name = format_ident!("target_{}", target_snake);
            target_fields.push(quote! {
                #target_field_name: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<#target_type>>>
            });
        }
    }

    // Find transitions from this state
    let transition_methods: Vec<_> = transitions
        .iter()
        .filter(|t| t.sources.iter().any(|s| s == state_name))
        .map(|t| {
            generate_queue_handle_transition_arc(
                col_prefix, key_name, t, state_map, error_name,
            )
        })
        .collect();

    // Generate accessor methods for state fields (including key)
    let mut field_accessors = vec![quote! {
        pub fn #key_name(&self) -> &#key_type {
            &self.item.as_ref().unwrap().#key_name
        }
    }];

    for f in &state.fields {
        let fname = &f.name;
        let fty = &f.ty;
        field_accessors.push(quote! {
            pub fn #fname(&self) -> &#fty {
                &self.item.as_ref().unwrap().#fname
            }
        });
    }

    quote! {
        pub struct #handle_name {
            #source_field,
            #(#target_fields,)*
            item: Option<#state_type>,
        }

        impl #handle_name {
            #(#field_accessors)*

            #(#transition_methods)*
        }

        impl Drop for #handle_name {
            fn drop(&mut self) {
                // If item wasn't consumed by a transition, put it back at front
                if let Some(item) = self.item.take() {
                    self.#source_field_name.lock().unwrap().push_front(item);
                }
            }
        }
    }
}

fn generate_queue_handle_transition_arc(
    col_prefix: &str,
    key_name: &syn::Ident,
    transition: &TypedTransition,
    state_map: &HashMap<String, &TypedState>,
    error_name: &syn::Ident,
) -> TokenStream2 {
    let method_name = &transition.name;
    let target = &transition.target;
    let target_type = prefixed_state_name(col_prefix, target);
    let target_snake = to_snake_case(&target.to_string());
    let target_field = format_ident!("target_{}", target_snake);

    let param_decls: Vec<_> = transition
        .params
        .iter()
        .map(|p| {
            let pname = &p.name;
            let pty = &p.ty;
            quote! { #pname: #pty }
        })
        .collect();

    let target_state = state_map.get(&target.to_string());

    // Build target construction (with key field)
    let target_construction = build_target_construction_with_key(
        &target_type,
        target_state,
        &transition.field_assignments,
        key_name,
    );

    // Guard check
    let guard_check = if let Some(guard) = &transition.guard {
        let rewritten = rewrite_self_to_item(quote! { #guard });
        quote! {
            if !(#rewritten) {
                return Err(#error_name::GuardFailed);
            }
        }
    } else {
        quote! {}
    };

    quote! {
        pub fn #method_name(mut self, #(#param_decls),*) -> Result<(), #error_name> {
            let item = self.item.as_ref().unwrap();
            #guard_check
            let item = self.item.take().unwrap();
            self.#target_field.lock().unwrap().push_back(#target_construction);
            Ok(())
        }
    }
}

fn generate_map_store_methods(
    col_name: &syn::Ident,
    col_prefix: &str,
    key_type: &syn::Type,
    states: &[TypedState],
    transitions: &[TypedTransition],
) -> TokenStream2 {
    let initial_state = &states[0];
    let initial_type = prefixed_state_name(col_prefix, &initial_state.name);
    let initial_field = format_ident!(
        "{}_{}",
        col_name,
        to_snake_case(&initial_state.name.to_string())
    );
    let insert_method = format_ident!("{}_insert", col_name);

    let initial_value = if initial_state.fields.is_empty() {
        quote! { #initial_type }
    } else {
        quote! { #initial_type::default() }
    };

    // Contains check across all states
    let contains_checks: Vec<_> = states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! { self.#field_name.lock().unwrap().contains_key(key) }
        })
        .collect();

    let contains_method = format_ident!("{}_contains", col_name);

    // Remove from any state
    let remove_checks: Vec<_> = states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! {
                if self.#field_name.lock().unwrap().remove(key).is_some() {
                    return true;
                }
            }
        })
        .collect();

    let remove_method = format_ident!("{}_remove", col_name);

    // Len across all states
    let len_adds: Vec<_> = states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! { self.#field_name.lock().unwrap().len() }
        })
        .collect();

    let len_method = format_ident!("{}_len", col_name);

    // Per-state access methods
    let state_methods: Vec<_> = states
        .iter()
        .map(|state| {
            let state_name = &state.name;
            let snake_state = to_snake_case(&state_name.to_string());
            let field_name = format_ident!("{}_{}", col_name, snake_state);
            let handle_name = format_ident!("{}{}Handle", col_prefix, state_name);
            let access_method = format_ident!("{}_{}", col_name, snake_state);
            let len_method = format_ident!("{}_{}_len", col_name, snake_state);
            let keys_method = format_ident!("{}_{}_keys", col_name, snake_state);

            // Collect target collections needed for this state's transitions
            let target_states: Vec<_> = transitions
                .iter()
                .filter(|t| t.sources.iter().any(|s| s == state_name))
                .map(|t| &t.target)
                .collect();

            let mut target_field_inits = Vec::new();
            let mut seen_targets = std::collections::HashSet::new();
            for target in &target_states {
                let target_str = target.to_string();
                if seen_targets.insert(target_str.clone()) {
                    let target_snake = to_snake_case(&target_str);
                    let target_store_field = format_ident!("{}_{}", col_name, target_snake);
                    let target_handle_field = format_ident!("target_{}", target_snake);
                    target_field_inits.push(quote! {
                        #target_handle_field: self.#target_store_field.clone()
                    });
                }
            }

            let source_handle_field = format_ident!("source_{}", snake_state);

            quote! {
                /// Get a handle to an item in this state.
                pub fn #access_method(&self, key: &#key_type) -> Option<#handle_name> {
                    let item = self.#field_name.lock().unwrap().remove(key)?;
                    Some(#handle_name {
                        #source_handle_field: self.#field_name.clone(),
                        #(#target_field_inits,)*
                        key: key.clone(),
                        item: Some(item),
                    })
                }

                /// Count items in this state.
                pub fn #len_method(&self) -> usize {
                    self.#field_name.lock().unwrap().len()
                }

                /// Get keys of items in this state.
                pub fn #keys_method(&self) -> Vec<#key_type>
                where
                    #key_type: Clone,
                {
                    self.#field_name.lock().unwrap().keys().cloned().collect()
                }
            }
        })
        .collect();

    quote! {
        /// Insert a new item in the initial state.
        pub fn #insert_method(&self, key: #key_type) -> bool {
            if self.#contains_method(&key) {
                return false;
            }
            self.#initial_field.lock().unwrap().insert(key, #initial_value);
            true
        }

        /// Check if an item exists in any state.
        pub fn #contains_method(&self, key: &#key_type) -> bool {
            #(#contains_checks)||*
        }

        /// Remove an item from any state.
        pub fn #remove_method(&self, key: &#key_type) -> bool {
            #(#remove_checks)*
            false
        }

        /// Get total count across all states.
        pub fn #len_method(&self) -> usize {
            #(#len_adds)+*
        }

        #(#state_methods)*
    }
}

fn generate_queue_store_methods(
    col_name: &syn::Ident,
    col_prefix: &str,
    key_name: &syn::Ident,
    key_type: &syn::Type,
    states: &[TypedState],
    transitions: &[TypedTransition],
) -> TokenStream2 {
    let initial_state = &states[0];
    let initial_type = prefixed_state_name(col_prefix, &initial_state.name);
    let initial_field = format_ident!(
        "{}_{}",
        col_name,
        to_snake_case(&initial_state.name.to_string())
    );
    let next_id_field = format_ident!("{}_next_id", col_name);
    let push_method = format_ident!("{}_push", col_name);

    // Len across all states
    let len_adds: Vec<_> = states
        .iter()
        .map(|s| {
            let field_name = format_ident!("{}_{}", col_name, to_snake_case(&s.name.to_string()));
            quote! { self.#field_name.lock().unwrap().len() }
        })
        .collect();

    let len_method = format_ident!("{}_len", col_name);

    // Per-state access methods
    let state_methods: Vec<_> = states
        .iter()
        .map(|state| {
            let state_name = &state.name;
            let state_type = prefixed_state_name(col_prefix, state_name);
            let snake_state = to_snake_case(&state_name.to_string());
            let field_name = format_ident!("{}_{}", col_name, snake_state);
            let handle_name = format_ident!("{}{}Handle", col_prefix, state_name);
            let next_method = format_ident!("{}_next_{}", col_name, snake_state);
            let len_method = format_ident!("{}_{}_len", col_name, snake_state);

            // Collect target collections needed for this state's transitions
            let target_states: Vec<_> = transitions
                .iter()
                .filter(|t| t.sources.iter().any(|s| s == state_name))
                .map(|t| &t.target)
                .collect();

            let mut target_field_inits = Vec::new();
            let mut seen_targets = std::collections::HashSet::new();
            for target in &target_states {
                let target_str = target.to_string();
                if seen_targets.insert(target_str.clone()) {
                    let target_snake = to_snake_case(&target_str);
                    let target_store_field = format_ident!("{}_{}", col_name, target_snake);
                    let target_handle_field = format_ident!("target_{}", target_snake);
                    target_field_inits.push(quote! {
                        #target_handle_field: self.#target_store_field.clone()
                    });
                }
            }

            let source_handle_field = format_ident!("source_{}", snake_state);

            let peek_method = format_ident!("{}_peek_{}", col_name, snake_state);

            quote! {
                /// Get a handle to the front item in this state.
                pub fn #next_method(&self) -> Option<#handle_name> {
                    let item = self.#field_name.lock().unwrap().pop_front()?;
                    Some(#handle_name {
                        #source_handle_field: self.#field_name.clone(),
                        #(#target_field_inits,)*
                        item: Some(item),
                    })
                }

                /// Peek at the front item (cloned).
                pub fn #peek_method(&self) -> Option<#state_type>
                where
                    #state_type: Clone,
                {
                    self.#field_name.lock().unwrap().front().cloned()
                }

                /// Count items in this state.
                pub fn #len_method(&self) -> usize {
                    self.#field_name.lock().unwrap().len()
                }
            }
        })
        .collect();

    // Initial value construction
    let initial_fields: Vec<_> = initial_state
        .fields
        .iter()
        .map(|f| {
            let fname = &f.name;
            quote! { #fname: Default::default() }
        })
        .collect();

    let initial_construction = if initial_fields.is_empty() {
        quote! { #initial_type { #key_name: id } }
    } else {
        quote! { #initial_type { #key_name: id, #(#initial_fields),* } }
    };

    quote! {
        /// Push a new item in the initial state.
        pub fn #push_method(&self) -> #key_type {
            let mut next_id = self.#next_id_field.lock().unwrap();
            let id = *next_id;
            *next_id = next_id.wrapping_add(1);
            self.#initial_field.lock().unwrap().push_back(#initial_construction);
            id
        }

        /// Get total count across all states.
        pub fn #len_method(&self) -> usize {
            #(#len_adds)+*
        }

        #(#state_methods)*
    }
}

fn build_target_construction(
    target_type: &syn::Ident,
    target_state: Option<&&TypedState>,
    field_assignments: &[FieldAssignment],
) -> TokenStream2 {
    if let Some(state) = target_state {
        if state.fields.is_empty() {
            quote! { #target_type }
        } else if field_assignments.is_empty() {
            quote! { #target_type::default() }
        } else {
            let field_inits: Vec<_> = field_assignments
                .iter()
                .map(|fa| {
                    let field = &fa.field;
                    let value = &fa.value;
                    let rewritten = rewrite_self_to_item(quote! { #value });
                    quote! { #field: #rewritten }
                })
                .collect();
            quote! { #target_type { #(#field_inits),* } }
        }
    } else {
        quote! { #target_type }
    }
}

fn build_target_construction_with_key(
    target_type: &syn::Ident,
    target_state: Option<&&TypedState>,
    field_assignments: &[FieldAssignment],
    key_name: &syn::Ident,
) -> TokenStream2 {
    let key_init = quote! { #key_name: item.#key_name };

    if let Some(state) = target_state {
        if state.fields.is_empty() {
            quote! { #target_type { #key_init } }
        } else if field_assignments.is_empty() {
            let default_fields: Vec<_> = state
                .fields
                .iter()
                .map(|f| {
                    let fname = &f.name;
                    quote! { #fname: Default::default() }
                })
                .collect();
            quote! { #target_type { #key_init, #(#default_fields),* } }
        } else {
            let field_inits: Vec<_> = field_assignments
                .iter()
                .map(|fa| {
                    let field = &fa.field;
                    let value = &fa.value;
                    let rewritten = rewrite_self_to_item(quote! { #value });
                    quote! { #field: #rewritten }
                })
                .collect();
            quote! { #target_type { #key_init, #(#field_inits),* } }
        }
    } else {
        quote! { #target_type { #key_init } }
    }
}

/// Rewrite `self.field` to `item.field` in token stream.
fn rewrite_self_to_item(tokens: TokenStream2) -> TokenStream2 {
    use proc_macro2::TokenTree;

    let mut result = Vec::new();
    let mut iter = tokens.into_iter().peekable();

    while let Some(token) = iter.next() {
        match &token {
            TokenTree::Ident(ident) if ident == "self" => {
                if let Some(TokenTree::Punct(p)) = iter.peek() {
                    if p.as_char() == '.' {
                        result.push(TokenTree::Ident(proc_macro2::Ident::new(
                            "item",
                            ident.span(),
                        )));
                        continue;
                    }
                }
                result.push(token);
            }
            TokenTree::Group(group) => {
                let inner = rewrite_self_to_item(group.stream());
                let mut new_group = proc_macro2::Group::new(group.delimiter(), inner);
                new_group.set_span(group.span());
                result.push(TokenTree::Group(new_group));
            }
            _ => {
                result.push(token);
            }
        }
    }

    result.into_iter().collect()
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_uppercase().next().unwrap());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}
