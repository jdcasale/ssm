//! Parsing for state machine syntax.
//!
//! Parses:
//! ```ignore
//! state_machine! {
//!     pub enum Name {
//!         #[initial]
//!         State1,
//!         State2 { field: Type },
//!     }
//!
//!     impl Name {
//!         fn transition(self: Source) -> Target { body }
//!
//!         #[guard(expr)]
//!         fn guarded(self: Source, arg: Type) -> Target { body }
//!     }
//! }
//! ```

use proc_macro2::{Span, TokenStream as TokenStream2};
use syn::{
    braced,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    token, Attribute, Error, Expr, Ident, Result, Token, Type, Visibility,
};

/// Complete state machine definition.
#[derive(Debug)]
pub struct StateMachine {
    pub vis: Visibility,
    pub name: Ident,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    pub hooks: Vec<LifecycleHook>,
    pub context: Option<ContextDef>,
}

/// A lifecycle hook (on_enter or on_exit).
#[derive(Debug)]
pub struct LifecycleHook {
    pub kind: HookKind,
    pub state: Ident,
    pub name: Ident,
    pub body: TokenStream2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HookKind {
    OnEnter,
    OnExit,
}


/// A state definition from the enum.
#[derive(Debug, Clone)]
pub struct State {
    pub name: Ident,
    pub fields: Vec<StateField>,
    pub is_initial: bool,
}

#[derive(Debug, Clone)]
pub struct StateField {
    pub name: Ident,
    pub ty: Type,
}

/// A transition from the impl block.
#[derive(Debug)]
pub struct Transition {
    pub name: Ident,
    pub source_states: Vec<Ident>,
    #[allow(dead_code)] // Parsed for potential future validation
    pub target_state: Ident,
    pub error_type: Option<Type>,  // Some if returns Result<State, E>
    pub args: Vec<(Ident, Type)>,
    pub guard: Option<Expr>,
    pub body: TokenStream2,
}

/// Optional context definition.
#[derive(Debug)]
pub struct ContextDef {
    pub ty: Type,
}

impl Parse for StateMachine {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut vis = Visibility::Inherited;
        let mut name: Option<Ident> = None;
        let mut states = Vec::new();
        let mut transitions = Vec::new();
        let mut hooks = Vec::new();
        let mut context = None;

        // Parse items until empty
        while !input.is_empty() {
            let _attrs = input.call(Attribute::parse_outer)?;

            if input.peek(Token![pub]) || input.peek(Token![enum]) {
                // Parse enum
                let item_vis: Visibility = input.parse()?;
                input.parse::<Token![enum]>()?;
                let enum_name: Ident = input.parse()?;

                vis = item_vis;
                name = Some(enum_name);

                let content;
                braced!(content in input);

                // Parse variants
                let variants: Punctuated<EnumVariant, Token![,]> =
                    content.parse_terminated(EnumVariant::parse, Token![,])?;

                for variant in variants {
                    let is_initial = variant.attrs.iter().any(|a| a.path().is_ident("initial"));
                    states.push(State {
                        name: variant.name,
                        fields: variant.fields,
                        is_initial,
                    });
                }
            } else if input.peek(Token![impl]) {
                // Parse impl block
                input.parse::<Token![impl]>()?;
                let _impl_name: Ident = input.parse()?;

                let content;
                braced!(content in input);

                // Parse methods
                while !content.is_empty() {
                    let method_attrs = content.call(Attribute::parse_outer)?;

                    if content.peek(Token![fn]) {
                        // Check if this is a lifecycle hook
                        let on_enter = method_attrs.iter().find(|a| a.path().is_ident("on_enter"));
                        let on_exit = method_attrs.iter().find(|a| a.path().is_ident("on_exit"));

                        if let Some(attr) = on_enter {
                            let hook = parse_lifecycle_hook(&content, attr, HookKind::OnEnter)?;
                            hooks.push(hook);
                        } else if let Some(attr) = on_exit {
                            let hook = parse_lifecycle_hook(&content, attr, HookKind::OnExit)?;
                            hooks.push(hook);
                        } else {
                            let transition = parse_transition_fn(&content, method_attrs)?;
                            transitions.push(transition);
                        }
                    } else {
                        return Err(content.error("expected fn"));
                    }
                }
            } else if input.peek(Token![type]) {
                // Parse context type: `type Context = MyContext;`
                input.parse::<Token![type]>()?;
                let type_name: Ident = input.parse()?;
                if type_name != "Context" {
                    return Err(Error::new(type_name.span(), "expected `Context`"));
                }
                input.parse::<Token![=]>()?;
                let ty: Type = input.parse()?;
                input.parse::<Token![;]>()?;
                context = Some(ContextDef { ty });
            } else {
                return Err(input.error("expected enum, impl, or type Context"));
            }
        }

        let name = name.ok_or_else(|| Error::new(Span::call_site(), "missing enum definition"))?;

        // Validate initial state
        let initial_count = states.iter().filter(|s| s.is_initial).count();
        if initial_count == 0 {
            return Err(Error::new(name.span(), "must have exactly one #[initial] state"));
        }
        if initial_count > 1 {
            return Err(Error::new(name.span(), "can only have one #[initial] state"));
        }

        Ok(StateMachine {
            vis,
            name,
            states,
            transitions,
            hooks,
            context,
        })
    }
}

/// Enum variant with optional fields.
struct EnumVariant {
    attrs: Vec<Attribute>,
    name: Ident,
    fields: Vec<StateField>,
}

impl Parse for EnumVariant {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let name: Ident = input.parse()?;

        let fields = if input.peek(token::Brace) {
            let content;
            braced!(content in input);
            let fields: Punctuated<NamedField, Token![,]> =
                content.parse_terminated(NamedField::parse, Token![,])?;
            fields.into_iter().map(|f| StateField { name: f.name, ty: f.ty }).collect()
        } else {
            Vec::new()
        };

        Ok(EnumVariant { attrs, name, fields })
    }
}

struct NamedField {
    name: Ident,
    ty: Type,
}

impl Parse for NamedField {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;
        Ok(NamedField { name, ty })
    }
}

/// Parse a transition function.
fn parse_transition_fn(input: ParseStream, attrs: Vec<Attribute>) -> Result<Transition> {
    input.parse::<Token![fn]>()?;
    let name: Ident = input.parse()?;

    // Parse parameters
    let params_content;
    syn::parenthesized!(params_content in input);

    // First param should be `self: SourceState` or `self: (State1, State2)`
    params_content.parse::<Token![self]>()?;
    params_content.parse::<Token![:]>()?;
    let source_type: Type = params_content.parse()?;
    let source_states = parse_source_states(&source_type)?;

    // Parse remaining args
    let mut args = Vec::new();
    while params_content.peek(Token![,]) {
        params_content.parse::<Token![,]>()?;
        if params_content.is_empty() {
            break;
        }
        let arg_name: Ident = params_content.parse()?;
        params_content.parse::<Token![:]>()?;
        let arg_type: Type = params_content.parse()?;
        args.push((arg_name, arg_type));
    }

    // Parse return type
    input.parse::<Token![->]>()?;
    let target_type: Type = input.parse()?;
    let parsed_return = parse_target_state(&target_type)?;

    // Parse body
    let body_content;
    braced!(body_content in input);
    let body: TokenStream2 = body_content.parse()?;

    // Extract guard from attributes
    let guard = attrs
        .iter()
        .find(|a| a.path().is_ident("guard"))
        .map(|a| a.parse_args::<Expr>())
        .transpose()?;

    Ok(Transition {
        name,
        source_states,
        target_state: parsed_return.target_state,
        error_type: parsed_return.error_type,
        args,
        guard,
        body,
    })
}

/// Parse source states from self type.
/// Handles: `State`, `(State1, State2)`, `State1 | State2` (as tuple)
fn parse_source_states(ty: &Type) -> Result<Vec<Ident>> {
    match ty {
        Type::Path(path) => {
            if let Some(ident) = path.path.get_ident() {
                Ok(vec![ident.clone()])
            } else {
                Err(Error::new(ty.span(), "expected state name"))
            }
        }
        Type::Tuple(tuple) => {
            let mut states = Vec::new();
            for elem in &tuple.elems {
                states.extend(parse_source_states(elem)?);
            }
            Ok(states)
        }
        Type::Paren(paren) => parse_source_states(&paren.elem),
        _ => Err(Error::new(ty.span(), "expected state name or tuple")),
    }
}

/// Parsed return type info.
struct ParsedReturn {
    target_state: Ident,
    error_type: Option<Type>,
}

/// Parse target state from return type.
/// Handles: `State` or `Result<State, Error>`
fn parse_target_state(ty: &Type) -> Result<ParsedReturn> {
    match ty {
        Type::Path(path) => {
            // Check if it's Result<State, Error>
            if let Some(segment) = path.path.segments.last() {
                if segment.ident == "Result" {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        let mut iter = args.args.iter();
                        let ok_type = iter.next().ok_or_else(|| {
                            Error::new(ty.span(), "Result needs type arguments")
                        })?;
                        let err_type = iter.next().ok_or_else(|| {
                            Error::new(ty.span(), "Result needs error type")
                        })?;

                        let target_state = if let syn::GenericArgument::Type(t) = ok_type {
                            extract_ident(t)?
                        } else {
                            return Err(Error::new(ok_type.span(), "expected state type"));
                        };

                        let error_type = if let syn::GenericArgument::Type(t) = err_type {
                            t.clone()
                        } else {
                            return Err(Error::new(err_type.span(), "expected error type"));
                        };

                        return Ok(ParsedReturn {
                            target_state,
                            error_type: Some(error_type),
                        });
                    }
                }
            }

            // Simple state name
            if let Some(ident) = path.path.get_ident() {
                Ok(ParsedReturn {
                    target_state: ident.clone(),
                    error_type: None,
                })
            } else {
                Err(Error::new(ty.span(), "expected state name"))
            }
        }
        Type::Paren(paren) => parse_target_state(&paren.elem),
        _ => Err(Error::new(ty.span(), "expected state name")),
    }
}

fn extract_ident(ty: &Type) -> Result<Ident> {
    match ty {
        Type::Path(path) => {
            if let Some(ident) = path.path.get_ident() {
                Ok(ident.clone())
            } else {
                Err(Error::new(ty.span(), "expected identifier"))
            }
        }
        _ => Err(Error::new(ty.span(), "expected identifier")),
    }
}

/// Parse a lifecycle hook function.
/// Example: `#[on_enter(Open)] fn entering_open(&self) { ... }`
fn parse_lifecycle_hook(input: ParseStream, attr: &Attribute, kind: HookKind) -> Result<LifecycleHook> {
    // Get the state name from the attribute
    let state: Ident = attr.parse_args()?;

    // Parse function
    input.parse::<Token![fn]>()?;
    let name: Ident = input.parse()?;

    // Parse parameters - should be just (&self) or (&mut self)
    let params_content;
    syn::parenthesized!(params_content in input);

    // Accept &self or &mut self
    if params_content.peek(Token![&]) {
        params_content.parse::<Token![&]>()?;
        if params_content.peek(Token![mut]) {
            params_content.parse::<Token![mut]>()?;
        }
        params_content.parse::<Token![self]>()?;
    } else {
        return Err(params_content.error("expected &self or &mut self"));
    }

    // No return type expected (or ignore it)
    if input.peek(Token![->]) {
        input.parse::<Token![->]>()?;
        let _: Type = input.parse()?;
    }

    // Parse body
    let body_content;
    braced!(body_content in input);
    let body: TokenStream2 = body_content.parse()?;

    Ok(LifecycleHook {
        kind,
        state,
        name,
        body,
    })
}

