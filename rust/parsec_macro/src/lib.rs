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

/// Validates a parser-building expression at compile time. When the expression
/// uses only statically-recognizable combinators — the primitives,
/// `concat`/`ignore`/`choice`/`optional`/`repeat`, and the
/// transform/tagging/position combinators — it emits specialized Rust that
/// skips the `Ast` interpreter. Otherwise it falls back to the unchanged
/// runtime expression.
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
                        ::nimble_parsec_rs::__private::parse_integer(__consumed),
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
                        ::nimble_parsec_rs::__private::parse_integer(__consumed),
                    ));
                    __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                    __input = &__input[__i..];
                }})
            }
        }

        // -- integer_range -----------------------------------------------
        "integer_range" if args.len() == 2 => {
            let min_n = &args[0];
            let max = &args[1];
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Int(
                        ::nimble_parsec_rs::__private::parse_integer(__consumed),
                    ));
                }
            };
            Some(quote! {{
                let __min: usize = #min_n;
                let __max_opt: ::std::option::Option<usize> = #max;
                let __raw = __input.as_bytes();
                let mut __i = 0usize;
                while __i < __raw.len() {
                    if let ::std::option::Option::Some(__max) = __max_opt {
                        if __i >= __max { break; }
                    }
                    if __raw[__i].is_ascii_digit() { __i += 1; } else { break; }
                }
                if __i < __min {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "expected integer".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
                let __consumed = &__input[..__i];
                #push
                __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                __input = &__input[__i..];
            }})
        }

        // -- bytes --------------------------------------------------------
        "bytes" if args.len() == 1 => {
            let count = &args[0];
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Str(__consumed.to_string()));
                }
            };
            Some(quote! {{
                let __count: usize = #count;
                match __input.get(..__count) {
                    ::std::option::Option::Some(__consumed) => {
                        #push
                        __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                        __input = &__input[__count..];
                    }
                    ::std::option::Option::None => {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: ::std::format!("expected {} bytes", __count),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                }
            }})
        }

        // -- ignore -------------------------------------------------------
        "ignore" if args.len() == 1 => codegen_impl(&args[0], true),

        // -- concat -------------------------------------------------------
        "concat" if args.len() == 2 => {
            let left = codegen_impl(&args[0], ignored)?;
            let right = codegen_impl(&args[1], ignored)?;
            Some(quote! { #left #right })
        }

        // -- ascii_char ---------------------------------------------------
        "ascii_char" if args.len() == 1 => {
            let preds = vec_elements(&args[0])?;
            let cond = char_class_condition(&preds, &quote!(__b))?;
            let reason_preds = quote!(&[#(#preds),*]);
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Int(
                        ::nimble_parsec_rs::Integer::from(__b),
                    ));
                }
            };
            Some(quote! {{
                match __input.as_bytes().first() {
                    ::std::option::Option::Some(&__b) if __b <= 0x7f && (#cond) => {
                        #push
                        let __consumed = &__input[..1];
                        __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                        __input = &__input[1..];
                    }
                    _ => {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: ::nimble_parsec_rs::__private::ascii_char_reason(#reason_preds),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                }
            }})
        }

        // -- utf8_char ----------------------------------------------------
        "utf8_char" if args.len() == 1 => {
            let preds = vec_elements(&args[0])?;
            let cond = char_class_condition(&preds, &quote!(__c))?;
            let reason_preds = quote!(&[#(#preds),*]);
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Int(
                        ::nimble_parsec_rs::Integer::from(__c as u32),
                    ));
                }
            };
            Some(quote! {{
                match __input.chars().next() {
                    ::std::option::Option::Some(__c) if (#cond) => {
                        #push
                        let __consumed = &__input[..__c.len_utf8()];
                        __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                        __input = &__input[__c.len_utf8()..];
                    }
                    _ => {
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: ::nimble_parsec_rs::__private::utf8_char_reason(#reason_preds),
                            rest: __input,
                            cursor: __cursor,
                        });
                    }
                }
            }})
        }

        // -- ascii_string -------------------------------------------------
        "ascii_string" if args.len() == 3 => {
            let preds = vec_elements(&args[0])?;
            let cond = char_class_condition(&preds, &quote!(__b))?;
            let min = &args[1];
            let max = &args[2];
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Str(__consumed.to_string()));
                }
            };
            Some(quote! {{
                let __min: usize = #min;
                let __max_opt: ::std::option::Option<usize> = #max;
                let __raw = __input.as_bytes();
                let mut __taken = 0usize;
                let mut __i = 0usize;
                while __i < __raw.len() {
                    if let ::std::option::Option::Some(__max) = __max_opt {
                        if __taken >= __max { break; }
                    }
                    let __b = __raw[__i];
                    if __b > 0x7f || !(#cond) { break; }
                    __i += 1;
                    __taken += 1;
                }
                if __taken < __min {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "expected ascii string with minimum length".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
                let __consumed = &__input[..__i];
                #push
                __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                __input = &__input[__i..];
            }})
        }

        // -- utf8_string --------------------------------------------------
        "utf8_string" if args.len() == 3 => {
            let preds = vec_elements(&args[0])?;
            let cond = char_class_condition(&preds, &quote!(__c))?;
            let min = &args[1];
            let max = &args[2];
            let push = if ignored {
                quote! {}
            } else {
                quote! {
                    __tokens.push(::nimble_parsec_rs::Value::Str(__consumed.to_string()));
                }
            };
            Some(quote! {{
                let __min: usize = #min;
                let __max_opt: ::std::option::Option<usize> = #max;
                let mut __consumed_end = 0usize;
                let mut __taken = 0usize;
                for (__idx, __c) in __input.char_indices() {
                    if let ::std::option::Option::Some(__max) = __max_opt {
                        if __taken >= __max { break; }
                    }
                    if !(#cond) { break; }
                    __consumed_end = __idx + __c.len_utf8();
                    __taken += 1;
                }
                if __taken < __min {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "expected utf8 string with minimum length".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
                let __consumed = &__input[..__consumed_end];
                #push
                __cursor = ::nimble_parsec_rs::__private::advance_cursor(__cursor, __consumed);
                __input = &__input[__consumed_end..];
            }})
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

        // -- transforms ---------------------------------------------------
        // Each runs its inner with `ignored = false` (real tokens, preserving
        // the inner's validations/effects), then operates on the tail it pushed
        // to `__tokens` and emits its own result only when not discarded.
        "tag" if args.len() == 2 => {
            let name = &args[0];
            let inner = codegen_impl(&args[1], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start);
                    let __name: ::std::sync::Arc<str> = (#name).into();
                    __tokens.push(::nimble_parsec_rs::Value::Tagged(
                        ::std::string::ToString::to_string(&__name),
                        __drained,
                    ));
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        "unwrap_and_tag" if args.len() == 2 => {
            let name = &args[0];
            let inner = codegen_impl(&args[1], false)?;
            let finish = if ignored {
                quote! {}
            } else {
                quote! {
                    let __value = __drained.pop().expect("length checked above");
                    __tokens.push(::nimble_parsec_rs::Value::KeyValue(
                        ::std::string::ToString::to_string(&__name),
                        ::std::boxed::Box::new(__value),
                    ));
                }
            };
            Some(quote! {{
                let __pre_input = __input;
                let __pre_cursor = __cursor;
                let __start = __tokens.len();
                #inner
                let mut __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                    __tokens.split_off(__start);
                let __name: ::std::sync::Arc<str> = (#name).into();
                if __drained.len() != 1 {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: ::std::format!(
                            "expected exactly one token to unwrap_and_tag as \"{}\"",
                            __name
                        ),
                        rest: __pre_input,
                        cursor: __pre_cursor,
                    });
                }
                #finish
            }})
        }

        "wrap" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start);
                    __tokens.push(::nimble_parsec_rs::Value::List(__drained));
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        "replace" if args.len() == 2 => {
            let value = &args[1];
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! {}
            } else {
                quote! { __tokens.push(#value); }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                __tokens.truncate(__start);
                #emit
            }})
        }

        "map" if args.len() == 2 => {
            let f = &args[1];
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __mapped: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start).into_iter().map(#f).collect();
                    __tokens.extend(__mapped);
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        "reduce" if args.len() == 2 => {
            let f = &args[1];
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start);
                    let __reduced = ::nimble_parsec_rs::__private::reduce_with(#f, __drained);
                    __tokens.push(__reduced);
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        "byte_offset" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start);
                    __tokens.push(::nimble_parsec_rs::Value::List(::std::vec![
                        ::nimble_parsec_rs::Value::List(__drained),
                        ::nimble_parsec_rs::Value::Int(::nimble_parsec_rs::Integer::from(
                            __cursor.byte_offset,
                        )),
                    ]));
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        "line" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], false)?;
            let emit = if ignored {
                quote! { __tokens.truncate(__start); }
            } else {
                quote! {
                    let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        __tokens.split_off(__start);
                    let __pos = ::nimble_parsec_rs::Value::List(::std::vec![
                        ::nimble_parsec_rs::Value::Int(::nimble_parsec_rs::Integer::from(
                            __cursor.line,
                        )),
                        ::nimble_parsec_rs::Value::Int(::nimble_parsec_rs::Integer::from(
                            __cursor.line_start_offset,
                        )),
                    ]);
                    __tokens.push(::nimble_parsec_rs::Value::List(::std::vec![
                        ::nimble_parsec_rs::Value::List(__drained),
                        __pos,
                    ]));
                }
            };
            Some(quote! {{
                let __start = __tokens.len();
                #inner
                #emit
            }})
        }

        // -- optional -----------------------------------------------------
        // Runs the inner in a closure so its `return Err` is caught here and
        // turned into "succeed, consuming nothing", matching the interpreter.
        "optional" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                let __opt_input = __input;
                let __opt_cursor = __cursor;
                let __opt_ctx = __context.clone();
                let __r: ::nimble_parsec_rs::ParseResult = (|| {
                    let mut __input = __opt_input;
                    let mut __cursor = __opt_cursor;
                    let mut __context = __opt_ctx.clone();
                    let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        ::std::vec::Vec::new();
                    #inner
                    Ok(::nimble_parsec_rs::ParseSuccess {
                        tokens: __tokens,
                        rest: __input,
                        cursor: __cursor,
                        context: __context,
                    })
                })();
                if let Ok(__ok) = __r {
                    __tokens.extend(__ok.tokens);
                    __input = __ok.rest;
                    __cursor = __ok.cursor;
                    __context = __ok.context;
                }
            }})
        }

        // -- repeat -------------------------------------------------------
        // Each iteration runs the inner in a closure (to catch failure); a
        // non-consuming success stops the loop, and `min` is enforced after,
        // matching the interpreter (the inner error propagates below `min`).
        "repeat" if args.len() == 3 => {
            let min = &args[1];
            let max = &args[2];
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                let __min: usize = #min;
                let __max_opt: ::std::option::Option<usize> = #max;
                let mut __count: usize = 0;
                loop {
                    if let ::std::option::Option::Some(__max) = __max_opt {
                        if __count >= __max {
                            break;
                        }
                    }
                    let __it_input = __input;
                    let __it_cursor = __cursor;
                    let __it_ctx = __context.clone();
                    let __r: ::nimble_parsec_rs::ParseResult = (|| {
                        let mut __input = __it_input;
                        let mut __cursor = __it_cursor;
                        let mut __context = __it_ctx.clone();
                        let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                            ::std::vec::Vec::new();
                        #inner
                        Ok(::nimble_parsec_rs::ParseSuccess {
                            tokens: __tokens,
                            rest: __input,
                            cursor: __cursor,
                            context: __context,
                        })
                    })();
                    match __r {
                        Ok(__ok) => {
                            if __ok.rest.len() == __input.len() {
                                break;
                            }
                            __tokens.extend(__ok.tokens);
                            __input = __ok.rest;
                            __cursor = __ok.cursor;
                            __context = __ok.context;
                            __count += 1;
                        }
                        Err(__e) => {
                            if __count < __min {
                                return Err(__e);
                            }
                            break;
                        }
                    }
                }
                if __count < __min {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "repeat did not reach minimum repetitions".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
            }})
        }

        // -- duplicate ----------------------------------------------------
        // Exactly N sequential repetitions; any failure aborts (the inner's
        // `return Err` propagates), so no recovery closure is needed.
        "duplicate" if args.len() == 2 => {
            let n = &args[1];
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                for _ in 0..#n {
                    #inner
                }
            }})
        }

        // -- eventually ---------------------------------------------------
        // Skip one codepoint at a time until the inner matches (run in a
        // closure to catch its failure); the skipped prefix is discarded.
        "eventually" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                let __ev_input = __input;
                let __ev_cursor = __cursor;
                loop {
                    let __try_input = __input;
                    let __try_cursor = __cursor;
                    let __try_ctx = __context.clone();
                    let __r: ::nimble_parsec_rs::ParseResult = (|| {
                        let mut __input = __try_input;
                        let mut __cursor = __try_cursor;
                        let mut __context = __try_ctx;
                        let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                            ::std::vec::Vec::new();
                        #inner
                        Ok(::nimble_parsec_rs::ParseSuccess {
                            tokens: __tokens,
                            rest: __input,
                            cursor: __cursor,
                            context: __context,
                        })
                    })();
                    if let Ok(__ok) = __r {
                        __tokens.extend(__ok.tokens);
                        __input = __ok.rest;
                        __cursor = __ok.cursor;
                        __context = __ok.context;
                        break;
                    }
                    match __input.chars().next() {
                        ::std::option::Option::Some(__ch) => {
                            let __consumed = &__input[..__ch.len_utf8()];
                            __cursor = ::nimble_parsec_rs::__private::advance_cursor(
                                __cursor, __consumed,
                            );
                            __input = &__input[__ch.len_utf8()..];
                        }
                        ::std::option::Option::None => {
                            return Err(::nimble_parsec_rs::ParseFailure {
                                reason: "expected combinator to eventually match".to_string(),
                                rest: __ev_input,
                                cursor: __ev_cursor,
                            });
                        }
                    }
                }
            }})
        }

        // -- repeat_while -------------------------------------------------
        // Like `repeat`, but the predicate (run via the eval_while helper to
        // pin its parameter types) gates each iteration, and an inner failure
        // simply stops the loop (it is not propagated below `min`).
        "repeat_while" if args.len() == 4 => {
            let while_fn = &args[1];
            let min = &args[2];
            let max = &args[3];
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                let __min: usize = #min;
                let __max_opt: ::std::option::Option<usize> = #max;
                let mut __count: usize = 0;
                loop {
                    if let ::std::option::Option::Some(__max) = __max_opt {
                        if __count >= __max {
                            break;
                        }
                    }
                    match ::nimble_parsec_rs::__private::eval_while(
                        #while_fn, __input, __cursor, &__context,
                    ) {
                        ::nimble_parsec_rs::RepeatWhileControl::Halt => break,
                        ::nimble_parsec_rs::RepeatWhileControl::Cont => {}
                    }
                    let __it_input = __input;
                    let __it_cursor = __cursor;
                    let __it_ctx = __context.clone();
                    let __r: ::nimble_parsec_rs::ParseResult = (|| {
                        let mut __input = __it_input;
                        let mut __cursor = __it_cursor;
                        let mut __context = __it_ctx;
                        let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                            ::std::vec::Vec::new();
                        #inner
                        Ok(::nimble_parsec_rs::ParseSuccess {
                            tokens: __tokens,
                            rest: __input,
                            cursor: __cursor,
                            context: __context,
                        })
                    })();
                    match __r {
                        Ok(__ok) => {
                            if __ok.rest.len() == __input.len() {
                                break;
                            }
                            __tokens.extend(__ok.tokens);
                            __input = __ok.rest;
                            __cursor = __ok.cursor;
                            __context = __ok.context;
                            __count += 1;
                        }
                        Err(_) => break,
                    }
                }
                if __count < __min {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "repeat_while did not reach minimum repetitions".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
            }})
        }

        // -- post_traverse / pre_traverse ---------------------------------
        // Run the inner (real tokens), then call the callback (via the
        // apply_traverse helper) with the drained tokens, threaded context, and
        // position (after for post, before for pre). The new context always
        // propagates; the new tokens are emitted only when not discarded.
        "post_traverse" | "pre_traverse" if args.len() == 2 => {
            let f = &args[1];
            let inner = codegen_impl(&args[0], false)?;
            let (pre_capture, position) = if name == "pre_traverse" {
                (quote!(let __pre_cursor = __cursor;), quote!(__pre_cursor))
            } else {
                (quote!(), quote!(__cursor))
            };
            let emit = if ignored {
                quote! { let _ = __new_tokens; }
            } else {
                quote! { __tokens.extend(__new_tokens); }
            };
            Some(quote! {{
                #pre_capture
                let __start = __tokens.len();
                #inner
                let __drained: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                    __tokens.split_off(__start);
                let (__new_tokens, __new_context) =
                    match ::nimble_parsec_rs::__private::apply_traverse(
                        #f, __drained, __context, #position,
                    ) {
                        Ok(__x) => __x,
                        Err(__reason) => {
                            return Err(::nimble_parsec_rs::ParseFailure {
                                reason: __reason,
                                rest: __input,
                                cursor: __cursor,
                            });
                        }
                    };
                __context = __new_context;
                #emit
            }})
        }

        // -- label --------------------------------------------------------
        // Runs the inner in a closure so its `return Err` is caught and the
        // reason rewritten to `expected <label>`, preserving the inner error's
        // position. Success passes through unchanged.
        "label" if args.len() == 2 => {
            let inner = codegen_impl(&args[0], ignored)?;
            let label = &args[1];
            Some(quote! {{
                let __lbl_input = __input;
                let __lbl_cursor = __cursor;
                let __lbl_ctx = __context.clone();
                let __r: ::nimble_parsec_rs::ParseResult = (move || {
                    let mut __input = __lbl_input;
                    let mut __cursor = __lbl_cursor;
                    let mut __context = __lbl_ctx;
                    let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        ::std::vec::Vec::new();
                    #inner
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
                    }
                    Err(__e) => {
                        let __lbl: ::std::sync::Arc<str> = (#label).into();
                        return Err(::nimble_parsec_rs::ParseFailure {
                            reason: ::std::format!("expected {}", __lbl),
                            rest: __e.rest,
                            cursor: __e.cursor,
                        });
                    }
                }
            }})
        }

        // -- lookahead ----------------------------------------------------
        // Zero-width: run the inner (tokens suppressed) in a closure; on success
        // consume/emit nothing; on failure propagate the inner error verbatim.
        "lookahead" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], true)?;
            Some(quote! {{
                let __la_input = __input;
                let __la_cursor = __cursor;
                let __la_ctx = __context.clone();
                let __r: ::nimble_parsec_rs::ParseResult = (move || {
                    let mut __input = __la_input;
                    let mut __cursor = __la_cursor;
                    let mut __context = __la_ctx;
                    let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        ::std::vec::Vec::new();
                    #inner
                    Ok(::nimble_parsec_rs::ParseSuccess {
                        tokens: __tokens,
                        rest: __input,
                        cursor: __cursor,
                        context: __context,
                    })
                })();
                // Propagate the inner failure verbatim; the Ok value (the peeked
                // parse) is discarded since the assertion consumes nothing.
                __r?;
            }})
        }

        // -- lookahead_not ------------------------------------------------
        // Zero-width negative: run the inner in a closure; if it matches, fail at
        // the original position; otherwise succeed consuming/emitting nothing.
        "lookahead_not" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], true)?;
            Some(quote! {{
                let __la_input = __input;
                let __la_cursor = __cursor;
                let __la_ctx = __context.clone();
                let __r: ::nimble_parsec_rs::ParseResult = (move || {
                    let mut __input = __la_input;
                    let mut __cursor = __la_cursor;
                    let mut __context = __la_ctx;
                    let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        ::std::vec::Vec::new();
                    #inner
                    Ok(::nimble_parsec_rs::ParseSuccess {
                        tokens: __tokens,
                        rest: __input,
                        cursor: __cursor,
                        context: __context,
                    })
                })();
                if __r.is_ok() {
                    return Err(::nimble_parsec_rs::ParseFailure {
                        reason: "did not expect lookahead parser to match".to_string(),
                        rest: __input,
                        cursor: __cursor,
                    });
                }
            }})
        }

        // -- debug --------------------------------------------------------
        // Prints the parser state to stderr around the inner (matching the
        // interpreter's format), then passes the result through. The closure lets
        // us print on the error path before propagating.
        "debug" if args.len() == 1 => {
            let inner = codegen_impl(&args[0], ignored)?;
            Some(quote! {{
                ::std::eprintln!("debug: parsing {:?} at {:?}", __input, __cursor);
                let __dbg_input = __input;
                let __dbg_cursor = __cursor;
                let __dbg_ctx = __context.clone();
                let __r: ::nimble_parsec_rs::ParseResult = (move || {
                    let mut __input = __dbg_input;
                    let mut __cursor = __dbg_cursor;
                    let mut __context = __dbg_ctx;
                    let mut __tokens: ::std::vec::Vec<::nimble_parsec_rs::Value> =
                        ::std::vec::Vec::new();
                    #inner
                    Ok(::nimble_parsec_rs::ParseSuccess {
                        tokens: __tokens,
                        rest: __input,
                        cursor: __cursor,
                        context: __context,
                    })
                })();
                match __r {
                    Ok(__ok) => {
                        ::std::eprintln!(
                            "debug: ok tokens={:?} rest={:?}",
                            __ok.tokens,
                            __ok.rest
                        );
                        __tokens.extend(__ok.tokens);
                        __input = __ok.rest;
                        __cursor = __ok.cursor;
                        __context = __ok.context;
                    }
                    Err(__e) => {
                        ::std::eprintln!("debug: error {:?}", __e.reason);
                        return Err(__e);
                    }
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

/// Splits a predicate expression like `AsciiPredicate::Range(b'0'..=b'9')` into
/// its variant name (last path segment) and single argument (if any).
fn predicate_parts(expr: &Expr) -> Option<(String, Option<Expr>)> {
    match expr {
        Expr::Path(p) => Some((p.path.segments.last()?.ident.to_string(), None)),
        Expr::Call(call) => {
            let Expr::Path(p) = call.func.as_ref() else {
                return None;
            };
            if call.args.len() != 1 {
                return None;
            }
            Some((
                p.path.segments.last()?.ident.to_string(),
                Some(call.args[0].clone()),
            ))
        }
        _ => None,
    }
}

/// Lowers a predicate list to a boolean membership test on `var`, mirroring the
/// runtime rule: hit at least one positive (or there are none) and no negative.
/// Returns `None` if any predicate is not a recognizable `*Predicate` variant.
fn char_class_condition(exprs: &[Expr], var: &TokenStream2) -> Option<TokenStream2> {
    let mut positives: Vec<TokenStream2> = Vec::new();
    let mut negatives: Vec<TokenStream2> = Vec::new();
    for e in exprs {
        let (name, inner) = predicate_parts(e)?;
        match (name.as_str(), inner) {
            ("Any", None) => positives.push(quote!(true)),
            ("Char", Some(c)) => positives.push(quote!(#var == #c)),
            ("NotChar", Some(c)) => negatives.push(quote!(#var == #c)),
            ("Range" | "NotRange", Some(r)) => {
                // Only literal closed ranges (`a..=b`) are recognized; lower them
                // to an opaque helper so the comparison dodges range/ascii lints.
                let Expr::Range(range) = &r else {
                    return None;
                };
                if !matches!(range.limits, syn::RangeLimits::Closed(_)) {
                    return None;
                }
                let (Some(lo), Some(hi)) = (&range.start, &range.end) else {
                    return None;
                };
                let cond = quote!(::nimble_parsec_rs::__private::in_range(#var, #lo, #hi));
                if name == "Range" {
                    positives.push(cond);
                } else {
                    negatives.push(cond);
                }
            }
            _ => return None,
        }
    }
    let pos = positives
        .into_iter()
        .reduce(|a, b| quote!(#a || #b))
        .unwrap_or(quote!(true));
    let neg = negatives
        .into_iter()
        .reduce(|a, b| quote!(#a || #b))
        .unwrap_or(quote!(false));
    Some(quote!((#pos) && !(#neg)))
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
