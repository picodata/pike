use proc_macro::{TokenStream, TokenTree};
use quote::quote;
use syn::{parse_macro_input, Item};

#[proc_macro_attribute]
pub fn picotest(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Fn(ref func) => {
            let func_name = &func.sig.ident;
            let block = &func.block;
            let inputs = &func.sig.inputs;

            let expanded = quote! {
                #[rstest]
                fn #func_name(#inputs) {
                    let cluster = picotest_helpers::run_cluster().unwrap();
                    #block
                }
            };

            TokenStream::from(expanded)
        }
        _ => {
            panic!("The picotest macro attribute is only valid when called on a function.");
        }
    }
}
