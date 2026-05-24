use proc_macro::TokenStream;
use quote::quote;
use syn::Expr;

/// Validates a parser-building expression at compile time and returns it
/// unchanged, so `compile_parser!(concat(...))` yields the same runtime
/// [`Parser`](nimble_parsec_rs::Parser) as writing the combinators directly.
///
/// Combinators are ordinary runtime functions (not compile-time data like
/// NimbleParsec's Elixir macros), so true specialization would require this
/// macro to parse the combinator token tree, recognize each combinator, and
/// emit generated parsing code that produces identical tokens. The reified
/// `Ast` in the main crate makes that tractable, but it remains future work;
/// today this macro is a validated passthrough to the runtime builders.
#[proc_macro]
pub fn compile_parser(input: TokenStream) -> TokenStream {
    let expr = syn::parse_macro_input!(input as Expr);
    quote!(#expr).into()
}
