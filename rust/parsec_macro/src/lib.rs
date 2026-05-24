use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Expr, Ident, Token};

// ---------------------------------------------------------------------------
// Shared parser for `name, expr` inputs used by the defXxx family
// ---------------------------------------------------------------------------

struct NamedParser {
    name: Ident,
    expr: Expr,
}

impl syn::parse::Parse for NamedParser {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let _comma: Token![,] = input.parse()?;
        let expr: Expr = input.parse()?;
        Ok(NamedParser { name, expr })
    }
}

// ---------------------------------------------------------------------------
// compile_parser!
// ---------------------------------------------------------------------------

/// Validates a parser-building expression at compile time.  When the
/// expression uses only statically-recognizable combinators (`string`,
/// `integer_exact`, `integer_min`, `ignore`, `concat`, `choice`, `empty`, `eos`),
/// emits specialized Rust that avoids intermediate token allocations.
/// Otherwise falls back to the unchanged runtime expression.
#[proc_macro]
pub fn compile_parser(input: TokenStream) -> TokenStream {
    let expr = parse_macro_input!(input as Expr);
    match codegen_impl(&expr, false) {
        Some(body) => quote! {
            ::nimble_parsec_rs::__private::native(|__input: &str, __cursor: ::nimble_parsec_rs::Cursor, __context: ::nimble_parsec_rs::Context| {
                let mut __input = __input;
                let mut __cursor = __cursor;
                let mut __context = __context;
                let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> = ::std::vec::Vec::new();
                #body
                Ok(::nimble_parsec_rs::ParseSuccess {
                    tokens: __tokens,
                    rest: __input,
                    cursor: __cursor,
                    context: __context,
                })
            })
        }
        .into(),
        None => quote!(#expr).into(),
    }
}

// ---------------------------------------------------------------------------
// defparsec! / defparsecp!
// ---------------------------------------------------------------------------

/// Defines a public parse function `pub fn name(input: &str) -> ParseResult<'_>`.
/// Uses generated specialized code when the combinator tree is fully
/// recognizable; otherwise caches the runtime `Parser` in a `OnceLock`.
#[proc_macro]
pub fn defparsec(input: TokenStream) -> TokenStream {
    named_parsec(input, true)
}

/// Like [`defparsec!`] but generates a private function (`fn`, not `pub fn`).
#[proc_macro]
pub fn defparsecp(input: TokenStream) -> TokenStream {
    named_parsec(input, false)
}

fn named_parsec(input: TokenStream, public: bool) -> TokenStream {
    let NamedParser { name, expr } = parse_macro_input!(input as NamedParser);
    let vis = if public { quote!(pub) } else { quote!() };

    match codegen_impl(&expr, false) {
        Some(body) => quote! {
            #vis fn #name(input: &str) -> ::nimble_parsec_rs::ParseResult<'_> {
                let mut __input = input;
                let mut __cursor = ::nimble_parsec_rs::Cursor::default();
                let mut __context = ::nimble_parsec_rs::Context::new();
                let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> = ::std::vec::Vec::new();
                #body
                Ok(::nimble_parsec_rs::ParseSuccess {
                    tokens: __tokens,
                    rest: __input,
                    cursor: __cursor,
                    context: __context,
                })
            }
        }
        .into(),
        None => quote! {
            #vis fn #name(input: &str) -> ::nimble_parsec_rs::ParseResult<'_> {
                static __PARSER: ::std::sync::OnceLock<::nimble_parsec_rs::Parser> =
                    ::std::sync::OnceLock::new();
                __PARSER.get_or_init(|| #expr).parse(input)
            }
        }
        .into(),
    }
}

// ---------------------------------------------------------------------------
// defcombinator! / defcombinatorp!
// ---------------------------------------------------------------------------

/// Defines a public function `pub fn name() -> Parser` that returns a
/// lazily-initialized, cached `Parser` built from the combinator expression.
#[proc_macro]
pub fn defcombinator(input: TokenStream) -> TokenStream {
    named_combinator(input, true)
}

/// Like [`defcombinator!`] but generates a private function.
#[proc_macro]
pub fn defcombinatorp(input: TokenStream) -> TokenStream {
    named_combinator(input, false)
}

fn named_combinator(input: TokenStream, public: bool) -> TokenStream {
    let NamedParser { name, expr } = parse_macro_input!(input as NamedParser);
    let vis = if public { quote!(pub) } else { quote!() };

    quote! {
        #vis fn #name() -> ::nimble_parsec_rs::Parser {
            static __PARSER: ::std::sync::OnceLock<::nimble_parsec_rs::Parser> =
                ::std::sync::OnceLock::new();
            ::std::clone::Clone::clone(__PARSER.get_or_init(|| #expr))
        }
    }
    .into()
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

/// Returns `Some(stmts)` when the entire expression tree is recognizable and
/// can be lowered to inline Rust; `None` means fall back to runtime.
///
/// `ignored` — when `true`, the current combinator's tokens are discarded
/// (it is wrapped, directly or transitively, by `ignore`).  Token-push code
/// is then suppressed, which is the primary allocation win over the interpreter.
fn codegen_impl(expr: &Expr, ignored: bool) -> Option<TokenStream2> {
    let (name, args) = call_parts(expr)?;

    match name.as_str() {
        // -- trivial primitives ------------------------------------------
        "empty" if args.is_empty() => Some(quote! {}),

        "eos" if args.is_empty() => Some(quote! {
            if !__input.is_empty() {
                return Err(::nimble_parsec_rs::ParseFailure {
                    reason: "expected end of string".to_string(),
                    rest: __input,
                    cursor: __cursor,
                });
            }
        }),

        // -- string -------------------------------------------------------
        "string" if args.len() == 1 => {
            let lit = &args[0];
            if ignored {
                Some(quote! {{
                    let __lit: &'static str = #lit;
                    if !__input.starts_with(__lit) {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: ::std::format!("expected string {:?}", __lit),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __lit);
                    __input = &__input[__lit.len()..];
                }})
            } else {
                Some(quote! {{
                    let __lit: &'static str = #lit;
                    match __input.strip_prefix(__lit) {
                        Some(__rest) => {
                            __tokens.push(::nimble_parsec_rs::Value::Str(__lit.to_string()));
                            __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __lit);
                            __input = __rest;
                        }
                        None => {
                            return Err(::nimble_parsec_rs::ParseFailure {
                                reason: ::std::format!("expected string {:?}", __lit),
                                rest: __input,
                                cursor: __cursor,
                            });
                        }
                    }
                }})
            }
        }

        // -- integer_exact -----------------------------------------------
        "integer_exact" if args.len() == 1 => {
            let n = &args[0];
            if ignored {
                Some(quote! {{
                    let __n: usize = #n;
                    let __raw = __input.as_bytes();
                    let mut __i = 0usize;
                    while __i < __raw.len() && __i < __n {
                        if __raw[__i].is_ascii_digit() { __i += 1; } else { break; }
                    }
                    if __i < __n {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: "expected integer".to_string(),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, &__input[..__n]);
                    __input = &__input[__n..];
                }})
            } else {
                Some(quote! {{
                    let __n: usize = #n;
                    let __raw = __input.as_bytes();
                    let mut __i = 0usize;
                    while __i < __raw.len() && __i < __n {
                        if __raw[__i].is_ascii_digit() { __i += 1; } else { break; }
                    }
                    if __i < __n {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: "expected integer".to_string(),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                    let __consumed = &__input[..__n];
                    __tokens.push(::nimble_parsec_rs::Value::Int(
                        __consumed.parse::<::nimble_parsec_rs::BigInt>().expect("digit run is valid"),
                    ));
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                    __input = &__input[__n..];
                }})
            }
        }

        // -- integer_min -------------------------------------------------
        "integer_min" if args.len() == 1 => {
            let min_n = &args[0];
            if ignored {
                Some(quote! {{
                    let __min: usize = #min_n;
                    let __raw = __input.as_bytes();
                    let mut __i = 0usize;
                    while __i < __raw.len() && __raw[__i].is_ascii_digit() { __i += 1; }
                    if __i < __min {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: "expected integer".to_string(),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, &__input[..__i]);
                    __input = &__input[__i..];
                }})
            } else {
                Some(quote! {{
                    let __min: usize = #min_n;
                    let __raw = __input.as_bytes();
                    let mut __i = 0usize;
                    while __i < __raw.len() && __raw[__i].is_ascii_digit() { __i += 1; }
                    if __i < __min {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: "expected integer".to_string(),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                    let __consumed = &__input[..__i];
                    __tokens.push(::nimble_parsec_rs::Value::Int(
                        __consumed.parse::<::nimble_parsec_rs::BigInt>().expect("digit run is valid"),
                    ));
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                    __input = &__input[__i..];
                }})
            }
        }

        // -- ignore -------------------------------------------------------
        "ignore" if args.len() == 1 => codegen_impl(&args[0], true),

        // -- concat -------------------------------------------------------
        "concat" if args.len() == 2 => {
            let left = codegen_impl(&args[0], ignored)?;
            let right = codegen_impl(&args[1], ignored)?;
            Some(quote! { #left #right })
        }

        // -- choice -------------------------------------------------------
        // Only when the argument is a `vec![..]` literal and every branch is
        // itself codegen-able; otherwise fall back to runtime. Each branch runs
        // in its own closure so its `return Err` backtracks to the next branch
        // instead of failing the whole parse, matching the interpreter (first
        // success wins; on total failure, branch reasons are joined with " or ").
        "choice" if args.len() == 1 => {
            let branches = vec_elements(&args[0])?;
            if branches.len() < 2 {
                return None;
            }
            let mut attempts = Vec::new();
            for branch in &branches {
                let body = codegen_impl(branch, ignored)?;
                attempts.push(quote! {
                    if !__choice_done {
                        let __r: ::nimble_parsec_rs::ParseResult = (|| {
                            let mut __input = __choice_input;
                            let mut __cursor = __choice_cursor;
                            let mut __context = __choice_ctx.clone();
                            let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                                ::std::vec::Vec::new();
                            #body
                            Ok(::nimble_parsec_rs::ParseSuccess {
                                tokens: __tokens,
                                rest: __input,
                                cursor: __cursor,
                                context: __context,
                            })
                        })();
                        match __r {
                            Ok(__ok) => {
                                __tokens.extend(__ok.tokens);
                                __input = __ok.rest;
                                __cursor = __ok.cursor;
                                __context = __ok.context;
                                __choice_done = true;
                            }
                            Err(__e) => __choice_reasons.push(__e.reason),
                        }
                    }
                });
            }
            Some(quote! {{
                let __choice_input = __input;
                let __choice_cursor = __cursor;
                let __choice_ctx = __context.clone();
                let mut __choice_reasons: ::std::vec::Vec<::std::string::String> =
                    ::std::vec::Vec::new();
                let mut __choice_done = false;
                #(#attempts)*
                if !__choice_done {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: __choice_reasons.join(" or "),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
            }})
        }

        _ => None,
    }
}

/// Extracts the element expressions from a `vec![..]` literal, or `None` if the
/// expression is not a `vec!` literal (e.g. a variable), which forces runtime
/// fallback.
fn vec_elements(expr: &Expr) -> Option<Vec<Expr>> {
    let Expr::Macro(m) = expr else {
        return None;
    };
    if !m.mac.path.is_ident("vec") {
        return None;
    }
    let parsed = m
        .mac
        .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .ok()?;
    Some(parsed.into_iter().collect())
}

/// Extracts `(last_path_segment_name, args)` from a function-call expression.
/// Handles both bare calls (`concat(...)`) and path-qualified ones
/// (`nimble_parsec_rs::concat(...)`).
fn call_parts(expr: &Expr) -> Option<(String, Vec<Expr>)> {
    let Expr::Call(call) = expr else {
        return None;
    };
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let name = path.path.segments.last()?.ident.to_string();
    let args: Vec<Expr> = call.args.iter().cloned().collect();
    Some((name, args))
}
