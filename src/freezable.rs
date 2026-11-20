use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data::Struct, DeriveInput, Fields::Named, Fields::Unnamed, Meta, Path, Token,
    parse_macro_input, punctuated::Punctuated,
};

fn has_serde_meta(attrs: &[Attribute], name: &str) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .any(|attr| {
            attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .map(|metas| metas.iter().any(|meta| meta.path().is_ident(name)))
                .unwrap_or(false)
        })
}

fn is_required_derive(path: &Path, required: &Path) -> bool {
    let Some(name) = required
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
    else {
        return false;
    };
    let path = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::");

    match name.as_str() {
        "Serialize" => matches!(
            path.as_str(),
            "Serialize"
                | "serde::Serialize"
                | "serde_derive::Serialize"
                | "freezable_trait::_serde::Serialize"
        ),
        "Deserialize" => matches!(
            path.as_str(),
            "Deserialize"
                | "serde::Deserialize"
                | "serde_derive::Deserialize"
                | "freezable_trait::_serde::Deserialize"
        ),
        "Clone" => matches!(
            path.as_str(),
            "Clone" | "std::clone::Clone" | "core::clone::Clone"
        ),
        _ => false,
    }
}

pub fn freezable_impl(_args: TokenStream, input: TokenStream) -> TokenStream {
    let DeriveInput {
        attrs,
        vis,
        ident,
        data,
        generics,
    } = parse_macro_input!(input as DeriveInput);

    match data {
        Struct(my_struct) => {
            match my_struct.fields {
                Named(fields) => {
                    let mut attrs_to_keep = Vec::new();
                    let mut derives = Punctuated::<Path, Token![,]>::new();
                    for attr in attrs {
                        if attr.path().is_ident("derive") {
                            let parsed = match attr.parse_args_with(
                                Punctuated::<Path, Token![,]>::parse_terminated,
                            ) {
                                Ok(parsed) => parsed,
                                Err(error) => return error.to_compile_error().into(),
                            };
                            derives.extend(parsed);
                        } else {
                            attrs_to_keep.push(attr);
                        }
                    }
                    let required_derives: [Path; 3] = [
                        syn::parse_quote!(freezable_trait::_serde::Serialize),
                        syn::parse_quote!(freezable_trait::_serde::Deserialize),
                        syn::parse_quote!(Clone),
                    ];
                    for derive in required_derives {
                        if !derives
                            .iter()
                            .any(|existing| is_required_derive(existing, &derive))
                        {
                            derives.push(derive);
                        }
                    }

                    let has_serde_crate = has_serde_meta(&attrs_to_keep, "crate");
                    let serde_crate_attr = if has_serde_crate {
                        quote!()
                    } else {
                        quote!(#[serde(crate = "freezable_trait::_serde")])
                    };
                    let serde_default_attr = if has_serde_meta(&attrs_to_keep, "default") {
                        quote!()
                    } else {
                        quote!(#[serde(default)])
                    };
                    let field_stream: Vec<proc_macro2::TokenStream> = fields
                        .named
                        .into_iter()
                        .map(|field| quote!(#field))
                        .collect();
                    let last_field = quote! {
                        #[serde(flatten)]
                        _unknown_fields: freezable_trait::_HashMap<String, freezable_trait::_serde_json::Value>
                    };
                    quote!{
                        #(#attrs_to_keep)*
                        #[derive(#derives)]
                        #serde_crate_attr
                        #serde_default_attr
                        #vis struct #ident #generics {
                            #(#field_stream),*,
                            #last_field
                        }
                    }
                },
                Unnamed(_fields) => panic!("Fields should be named"),
                _unit => panic!("Should not be unit struct"),
            }
        }
        _ => panic!("Must be a struct.")
    }.into()
}
