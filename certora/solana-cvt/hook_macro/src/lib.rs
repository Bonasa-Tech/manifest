use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, AttributeArgs, ItemFn};

/**
* This macro is used to insert a hook at the start of a function.
* # Example
* #[cvt_hook_start(hook())]
   fn t1() {
       // hook inserted here
       println!("t1");
   }

   expands to

   fn t1() {
       hook();
       println!("t1");
   }
*/
#[proc_macro_attribute]
pub fn cvt_hook_start(attr: TokenStream, input: TokenStream) -> TokenStream {
    // parse the attribute argument
    let attr = parse_macro_input!(attr as AttributeArgs);
    if attr.len() != 1 {
        return quote! {
            compile_error!("Expected 1 argument");
        }
        .into();
    }

    let arg = &attr[0];

    // parse the input tokens and make sure it is a function
    let mut fn_item = parse_macro_input!(input as ItemFn);

    // insert tokens_start into fn item statements at position 0
    let tokens_start = quote! { #arg; };

    fn_item
        .block
        .stmts
        .insert(0, syn::parse(tokens_start.into()).unwrap());

    fn_item.into_token_stream().into()
}

/**
* This macro is used to insert a hook at the end of a function.
* If the function returns a value, the hook is inserted before the return statement.
* # Example
* #[cvt_hook_end(hook())]
   fn t1() {
       assert_eq!(1, 1);
       assert_eq!(2, 2);
       // hook inserted here
   }

   expands to

   fn t1() {
       assert_eq!(1, 1);
       assert_eq!(2, 2);
       hook()
   }
*/
#[proc_macro_attribute]
pub fn cvt_hook_end(attr: TokenStream, input: TokenStream) -> TokenStream {
    // parse the attribute argument
    let attr = parse_macro_input!(attr as AttributeArgs);
    if attr.len() != 1 {
        return quote! {
            compile_error!("Expected 1 argument");
        }
        .into();
    }

    let arg = &attr[0];

    // parse the input tokens and make sure it is a function
    let mut fn_item = parse_macro_input!(input as ItemFn);
    let ret_type = &fn_item.sig.output;

    // create tokens_end
    let tokens_end = quote! { #arg; };

    // len of fn item statements
    let len = fn_item.block.stmts.len();

    match ret_type {
        syn::ReturnType::Default => {
            // insert tokens_end into fn item statements at position len
            fn_item
                .block
                .stmts
                .insert(len, syn::parse(tokens_end.into()).unwrap());
        }
        syn::ReturnType::Type(_, _) => {
            // insert tokens_end into fn item statements at position len-1
            fn_item
                .block
                .stmts
                .insert(len - 1, syn::parse(tokens_end.into()).unwrap());
        }
    }

    fn_item.into_token_stream().into()
}
