use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input,
    Data::{Enum, Struct, Union},
    DeriveInput,
    Fields::{self, Named, Unnamed},
    FieldsNamed, FieldsUnnamed, Ident,
};

fn of_named_fields(n: &Ident, named_fields: &FieldsNamed) -> proc_macro2::TokenStream {
    let initialize = named_fields.named.iter().map(|f| {
        let name = f.ident.as_ref().unwrap();
        quote! {
            #name: ::nondet::nondet(),
        }
    });

    quote! {
        #n {
            #( #initialize )*
        }
    }
}

fn of_unnamed_fields(n: &Ident, unnamed: &FieldsUnnamed) -> proc_macro2::TokenStream {
    let initialize = unnamed.unnamed.iter().map(|_| {
        quote! { ::nondet::nondet(), }
    });

    quote! {
        #n (
            #( #initialize )*
        )
    }
}

#[proc_macro_derive(Nondet)]
pub fn derive_nondet(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = input.ident;
    match input.data {
        Enum(_) => {
            todo!("Enum not supported yet")
        }

        Union(_) => {
            todo!("Union not supported yet")
        }

        Struct(ds) => match ds.fields {
            Fields::Unit => quote! {
                impl crate for #name {
                    fn nondet() -> #name {
                        ()
                    }
                }
            }
            .into(),

            Named(named) => {
                let init = of_named_fields(&name, &named);
                quote! {
                    impl ::nondet::Nondet for #name {
                        fn nondet() -> #name {
                            #init
                        }
                    }
                }
                .into()
            }

            Unnamed(fields) => {
                let init = of_unnamed_fields(&name, &fields);
                quote! {
                    impl ::nondet::Nondet for #name {
                        fn nondet() -> #name {
                            #init
                        }
                    }
                }
                .into()
            }
        },
    }
}
