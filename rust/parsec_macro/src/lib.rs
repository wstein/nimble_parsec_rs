use proc_macro::TokenStream;
use quote::quote;
use syn::Expr;

#[proc_macro]
pub fn compile_parser(input: TokenStream) -> TokenStream {
    let expr = syn::parse_macro_input!(input as Expr);

    // Phase-3 scaffold: this macro validates parser syntax at compile time and
    // returns the parser expression. Future phases will lower the expression
    // into specialized generated parsing code.
    quote!(#expr).into()
}
