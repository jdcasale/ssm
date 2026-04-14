//! Parsing for typed store! macro.
//!
//! Syntax:
//! ```ignore
//! store! {
//!     MessageStore {
//!         // Map with explicit key
//!         messages: Map<u64> {
//!             states {
//!                 Pending,
//!                 InFlight { leader: u64, term: u64 },
//!                 Acked,
//!             }
//!             transitions {
//!                 mark_in_flight(leader: u64, term: u64): Pending -> InFlight {
//!                     leader: leader,
//!                     term: term,
//!                 }
//!                 ack(): (InFlight, InDoubt) -> Acked;
//!             }
//!         }
//!
//!         // Queue with auto-generated key
//!         tasks: Queue {
//!             key id: Uuid,
//!             states { ... }
//!             transitions { ... }
//!         }
//!     }
//! }
//! ```

use syn::{
    braced, parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Expr, Ident, Result, Token, Type, Visibility,
};

/// Complete typed store definition.
#[derive(Debug)]
pub struct TypedStore {
    pub vis: Visibility,
    pub name: Ident,
    pub collections: Vec<TypedCollection>,
}

/// The kind of collection.
#[derive(Debug, Clone)]
pub enum CollectionKind {
    /// Map with explicit key type provided by caller.
    Map(Type),
    /// Queue with auto-generated key field.
    Queue { key_name: Ident, key_type: Type },
}

/// A collection with embedded FSM definition.
#[derive(Debug)]
pub struct TypedCollection {
    pub name: Ident,
    pub kind: CollectionKind,
    pub states: Vec<TypedState>,
    pub transitions: Vec<TypedTransition>,
}

/// A state in the FSM.
#[derive(Debug, Clone)]
pub struct TypedState {
    pub name: Ident,
    pub fields: Vec<TypedField>,
}

/// A field in a state.
#[derive(Debug, Clone)]
pub struct TypedField {
    pub name: Ident,
    pub ty: Type,
}

/// A transition in the FSM.
#[derive(Debug)]
pub struct TypedTransition {
    pub name: Ident,
    pub params: Vec<TypedField>,
    pub sources: Vec<Ident>,
    pub target: Ident,
    pub guard: Option<Expr>,
    pub field_assignments: Vec<FieldAssignment>,
}

/// A field assignment in a transition body.
#[derive(Debug)]
pub struct FieldAssignment {
    pub field: Ident,
    pub value: Expr,
}

impl Parse for TypedStore {
    fn parse(input: ParseStream) -> Result<Self> {
        let vis: Visibility = input.parse()?;
        let name: Ident = input.parse()?;

        let content;
        braced!(content in input);

        let mut collections = Vec::new();
        while !content.is_empty() {
            collections.push(content.parse()?);
        }

        Ok(TypedStore { vis, name, collections })
    }
}

impl Parse for TypedCollection {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;

        // Parse Map<KeyType> or Queue
        let kind_ident: Ident = input.parse()?;
        let kind = if kind_ident == "Map" {
            input.parse::<Token![<]>()?;
            let key_type: Type = input.parse()?;
            input.parse::<Token![>]>()?;
            CollectionKind::Map(key_type)
        } else if kind_ident == "Queue" {
            // Queue's key is defined inside the braces
            CollectionKind::Queue {
                key_name: Ident::new("id", kind_ident.span()),
                key_type: syn::parse_quote!(u64), // placeholder, parsed below
            }
        } else {
            return Err(syn::Error::new(kind_ident.span(), "expected Map or Queue"));
        };

        // Parse FSM definition in braces
        let fsm_content;
        braced!(fsm_content in input);

        let mut states = Vec::new();
        let mut transitions = Vec::new();
        let mut queue_key: Option<(Ident, Type)> = None;

        while !fsm_content.is_empty() {
            let section: Ident = fsm_content.parse()?;

            if section == "key" {
                // Parse: key id: Uuid,
                let key_name: Ident = fsm_content.parse()?;
                fsm_content.parse::<Token![:]>()?;
                let key_type: Type = fsm_content.parse()?;
                if fsm_content.peek(Token![,]) {
                    fsm_content.parse::<Token![,]>()?;
                }
                queue_key = Some((key_name, key_type));
            } else if section == "states" {
                let states_content;
                braced!(states_content in fsm_content);
                states = parse_states(&states_content)?;
            } else if section == "transitions" {
                let trans_content;
                braced!(trans_content in fsm_content);
                transitions = parse_transitions(&trans_content)?;
            } else {
                return Err(syn::Error::new(section.span(), "expected 'key', 'states', or 'transitions'"));
            }
        }

        // Update Queue kind with parsed key if present
        let kind = match kind {
            CollectionKind::Queue { .. } => {
                let (key_name, key_type) = queue_key.ok_or_else(|| {
                    syn::Error::new(kind_ident.span(), "Queue requires 'key' declaration")
                })?;
                CollectionKind::Queue { key_name, key_type }
            }
            other => other,
        };

        // Optional trailing comma
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }

        Ok(TypedCollection {
            name,
            kind,
            states,
            transitions,
        })
    }
}

fn parse_states(input: ParseStream) -> Result<Vec<TypedState>> {
    let mut states = Vec::new();

    while !input.is_empty() {
        let name: Ident = input.parse()?;

        let fields = if input.peek(syn::token::Brace) {
            let fields_content;
            braced!(fields_content in input);
            let fields: Punctuated<FieldDef, Token![,]> =
                fields_content.parse_terminated(FieldDef::parse, Token![,])?;
            fields.into_iter().map(|f| TypedField { name: f.name, ty: f.ty }).collect()
        } else {
            Vec::new()
        };

        states.push(TypedState { name, fields });

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    Ok(states)
}

struct FieldDef {
    name: Ident,
    ty: Type,
}

impl Parse for FieldDef {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;
        Ok(FieldDef { name, ty })
    }
}

fn parse_transitions(input: ParseStream) -> Result<Vec<TypedTransition>> {
    let mut transitions = Vec::new();

    while !input.is_empty() {
        transitions.push(parse_single_transition(input)?);
    }

    Ok(transitions)
}

fn parse_single_transition(input: ParseStream) -> Result<TypedTransition> {
    let name: Ident = input.parse()?;

    // Parse parameters
    let params_content;
    parenthesized!(params_content in input);
    let params: Punctuated<FieldDef, Token![,]> =
        params_content.parse_terminated(FieldDef::parse, Token![,])?;
    let params: Vec<TypedField> = params.into_iter()
        .map(|f| TypedField { name: f.name, ty: f.ty })
        .collect();

    input.parse::<Token![:]>()?;

    // Parse sources
    let sources = if input.peek(syn::token::Paren) {
        let sources_content;
        parenthesized!(sources_content in input);
        let sources: Punctuated<Ident, Token![,]> =
            sources_content.parse_terminated(Ident::parse, Token![,])?;
        sources.into_iter().collect()
    } else {
        vec![input.parse()?]
    };

    input.parse::<Token![->]>()?;

    let target: Ident = input.parse()?;

    // Optional guard
    let guard = if input.peek(Token![where]) {
        input.parse::<Token![where]>()?;
        Some(input.parse()?)
    } else {
        None
    };

    // Body or semicolon
    let field_assignments = if input.peek(syn::token::Brace) {
        let body_content;
        braced!(body_content in input);
        parse_field_assignments(&body_content)?
    } else {
        input.parse::<Token![;]>()?;
        Vec::new()
    };

    Ok(TypedTransition {
        name,
        params,
        sources,
        target,
        guard,
        field_assignments,
    })
}

fn parse_field_assignments(input: ParseStream) -> Result<Vec<FieldAssignment>> {
    let mut assignments = Vec::new();

    while !input.is_empty() {
        let field: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let value: Expr = input.parse()?;

        assignments.push(FieldAssignment { field, value });

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    Ok(assignments)
}
