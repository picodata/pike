mod utils;

use darling::ast::NestedMeta;
use darling::{Error, FromMeta};
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, parse_quote, Attribute, Item, Stmt};
use utils::traverse_use_item;

fn plugin_path_default() -> String {
    ".".to_string()
}
fn plugin_timeout_default() -> u8 {
    5
}
#[derive(Debug, FromMeta)]
struct PluginCfg {
    #[darling(default = "plugin_path_default")]
    path: String,
    #[darling(default = "plugin_timeout_default")]
    timeout: u8,
}

#[proc_macro_attribute]
pub fn picotest(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    let attr = match NestedMeta::parse_meta_list(attr.into()) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(Error::from(e).write_errors());
        }
    };

    let cfg = match PluginCfg::from_list(&attr) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };

    let path = cfg.path;
    let timeout = cfg.timeout;

    let rstest_macro: Attribute = parse_quote! { #[rstest] };
    let input = match input {
        Item::Fn(mut func) => {
            let run_cluster: Stmt = parse_quote! {
                let mut cluster = picotest_helpers::run_cluster(
                    #path,
                    #timeout,
                ).unwrap();
            };

            func.attrs.push(rstest_macro.clone());
            let mut stmts = vec![run_cluster];
            stmts.append(&mut func.block.stmts);
            func.block.stmts = stmts;
            Item::Fn(func)
        }
        Item::Mod(mut m) => {
            let (brace, items) = m.content.clone().unwrap();

            let run_cluster: Stmt = parse_quote! {
                let mut cluster = CLUSTER.get_or_init(|| {
                    picotest_helpers::run_cluster(#path, #timeout).unwrap()
                });
            };

            let stop_cluster: Stmt = parse_quote! {
                if TESTS_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    let mut cluster = CLUSTER.get().unwrap();
                    cluster.stop();
                    drop(cluster);
                }
            };
            let resume: Stmt = parse_quote! {
                if let Err(err) = result {
                    panic::resume_unwind(err);
                }
            };

            let mut has_once_lock: bool = false;
            let mut use_atomic_usize: bool = false;
            let mut use_atomic_ordering: bool = false;
            let mut use_panic: bool = false;
            let mut test_count: usize = 0;
            let mut e: Vec<Item> = items
                .into_iter()
                .map(|t| match t {
                    Item::Fn(mut func) => {
                        let func_name = &func.sig.ident;
                        if func_name.to_string().starts_with("test_") {
                            test_count += 1;
                            func.attrs.push(rstest_macro.clone());
                            let block = func.block.clone();
                            let body: Stmt = parse_quote! {
                                let result = panic::catch_unwind(|| {
                                    #block
                                });
                            };

                            func.block.stmts = vec![
                                run_cluster.clone(),
                                body,
                                stop_cluster.clone(),
                                resume.clone(),
                            ];
                            Item::Fn(func)
                        } else {
                            Item::Fn(func)
                        }
                    }
                    Item::Use(use_stmt) => {
                        if traverse_use_item(&use_stmt.tree, vec!["std", "sync", "OnceLock"])
                            .is_some()
                        {
                            has_once_lock = true;
                        }
                        if traverse_use_item(
                            &use_stmt.tree,
                            vec!["std", "sync", "atomic", "AtomicUsize"],
                        )
                        .is_some()
                        {
                            use_atomic_usize = true;
                        }
                        if traverse_use_item(
                            &use_stmt.tree,
                            vec!["std", "sync", "atomic", "Ordering"],
                        )
                        .is_some()
                        {
                            use_atomic_ordering = true;
                        }
                        if traverse_use_item(&use_stmt.tree, vec!["std", "panic"]).is_some() {
                            use_panic = true;
                        }
                        Item::Use(use_stmt)
                    }
                    e => e,
                })
                .collect();

            let mut use_content = vec![
                parse_quote!(
                    use picotest_helpers::Cluster;
                ),
                parse_quote!(
                    use rstest::*;
                ),
            ];
            if !has_once_lock {
                use_content.push(parse_quote!(
                    use std::sync::OnceLock;
                ));
            }

            if !use_atomic_usize {
                use_content.push(parse_quote!(
                    use std::sync::atomic::AtomicUsize;
                ));
            }
            if !use_atomic_ordering {
                use_content.push(parse_quote!(
                    use std::sync::atomic::Ordering;
                ));
            }

            if !use_panic {
                use_content.push(parse_quote!(
                    use std::panic;
                ));
            }

            use_content.push(parse_quote!(
                static CLUSTER: OnceLock<Cluster> = OnceLock::new();
            ));
            use_content.push(parse_quote!(
                static TESTS_COUNT: AtomicUsize = AtomicUsize::new(#test_count);
            ));
            use_content.append(&mut e);

            m.content = Some((brace, use_content));
            Item::Mod(m)
        }
        _ => {
            panic!("The picotest macro attribute is only valid when called on a function.");
        }
    };
    TokenStream::from(quote! (#input))
}
