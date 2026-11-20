use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::{
    Attribute, Data::Struct, DeriveInput, Expr, Fields::Named, Fields::Unnamed, Generics, Meta,
    Path, Token, Visibility, braced, parse::Parse, parse::ParseStream, punctuated::Punctuated,
};

struct FreezableField {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: syn::Ident,
    ty: syn::Type,
    default: Option<Expr>,
}

impl Parse for FreezableField {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let vis = input.parse()?;
        let ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;
        let default = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        Ok(Self {
            attrs,
            vis,
            ident,
            ty,
            default,
        })
    }
}

impl ToTokens for FreezableField {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let Self {
            attrs,
            vis,
            ident,
            ty,
            ..
        } = self;
        tokens.extend(quote!(#(#attrs)* #vis #ident: #ty));
    }
}

struct FreezableStruct {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: syn::Ident,
    generics: Generics,
    fields: Vec<FreezableField>,
}

impl Parse for FreezableStruct {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let vis = input.parse()?;
        input.parse::<Token![struct]>()?;
        let ident = input.parse()?;
        let mut generics: Generics = input.parse()?;
        generics.where_clause = input.parse()?;
        let content;
        braced!(content in input);
        let fields = Punctuated::<FreezableField, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        Ok(Self {
            attrs,
            vis,
            ident,
            generics,
            fields,
        })
    }
}

fn parse_freezable_struct(
    input: TokenStream2,
) -> syn::Result<(DeriveInput, Vec<(syn::Ident, Expr)>)> {
    let parsed = syn::parse2::<FreezableStruct>(input)?;
    let defaults = parsed
        .fields
        .iter()
        .filter_map(|field| {
            field
                .default
                .as_ref()
                .map(|value| (field.ident.clone(), value.clone()))
        })
        .collect();
    let FreezableStruct {
        attrs,
        vis,
        ident,
        generics,
        fields,
    } = parsed;
    let input = quote! {
        #(#attrs)*
        #vis struct #ident #generics {
            #(#fields),*
        }
    };
    Ok((syn::parse2(input)?, defaults))
}

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

fn conditional_cfg_metas(attrs: &[Meta]) -> Vec<Meta> {
    let mut conditional = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("cfg") {
            conditional.push(attr.clone());
            continue;
        }
        if !attr.path().is_ident("cfg_attr") {
            continue;
        }

        let Meta::List(list) = attr else {
            continue;
        };
        let Ok(metas) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            continue;
        };
        let mut metas = metas.into_iter();
        let Some(condition) = metas.next() else {
            continue;
        };
        let nested = conditional_cfg_metas(&metas.collect::<Vec<_>>());
        if !nested.is_empty() {
            conditional.push(syn::parse_quote!(cfg_attr(#condition, #(#nested),*)));
        }
    }
    conditional
}

fn field_configuration_attrs(attrs: &[Attribute]) -> Vec<TokenStream2> {
    let metas = attrs
        .iter()
        .map(|attr| attr.meta.clone())
        .collect::<Vec<_>>();
    conditional_cfg_metas(&metas)
        .iter()
        .map(|meta| quote!(#[#meta]))
        .collect()
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
    let (input, field_defaults) = match parse_freezable_struct(input.into()) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    let DeriveInput {
        attrs,
        vis,
        ident,
        data,
        generics,
    } = input;

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
                    let struct_configuration_attrs = field_configuration_attrs(&attrs_to_keep);
                    let last_field = quote! {
                        #[serde(flatten)]
                        _unknown_fields: freezable_trait::_HashMap<String, freezable_trait::_serde_json::Value>
                    };
                    let mut default_generics = generics.clone();
                    for field in &fields.named {
                        let name = field.ident.as_ref().expect("named field");
                        let has_explicit_default = field_defaults
                            .iter()
                            .any(|(field_name, _)| field_name == name);
                        let configuration_attrs = field_configuration_attrs(&field.attrs);
                        if !configuration_attrs.is_empty() && !has_explicit_default {
                            return syn::Error::new_spanned(
                                field,
                                "conditionally compiled fields must declare an explicit default",
                            )
                            .to_compile_error()
                            .into();
                        }
                        if has_explicit_default {
                            continue;
                        }
                        let ty = &field.ty;
                        let predicate: syn::WherePredicate =
                            syn::parse_quote!(#ty: ::core::default::Default);
                        let predicate_tokens = predicate.to_token_stream().to_string();
                        let where_clause = default_generics.make_where_clause();
                        if !where_clause.predicates.iter().any(|existing| {
                            existing.to_token_stream().to_string() == predicate_tokens
                        }) {
                            where_clause.predicates.push(predicate);
                        }
                    }
                    let default_values = fields.named.iter().map(|field| {
                        let name = field.ident.as_ref().expect("named field");
                        let configuration_attrs = field_configuration_attrs(&field.attrs);
                        if let Some((_, value)) = field_defaults.iter().find(|(field_name, _)| field_name == name) {
                            quote!(#(#configuration_attrs)* #name: (#value).into())
                        } else {
                            quote!(#(#configuration_attrs)* #name: ::core::default::Default::default())
                        }
                    });
                    let (impl_generics, ty_generics, where_clause) =
                        default_generics.split_for_impl();
                    let generated_default = quote! {
                        #(#struct_configuration_attrs)*
                        impl #impl_generics ::core::default::Default for #ident #ty_generics #where_clause {
                            fn default() -> Self {
                                Self {
                                    #(#default_values,)*
                                    _unknown_fields: ::core::default::Default::default(),
                                }
                            }
                        }
                    };
                    let field_stream: Vec<proc_macro2::TokenStream> = fields
                        .named
                        .into_iter()
                        .map(|field| quote!(#field))
                        .collect();
                    quote!{
                        #(#attrs_to_keep)*
                        #[derive(#derives)]
                        #serde_crate_attr
                        #serde_default_attr
                        #vis struct #ident #generics {
                            #(#field_stream),*,
                            #last_field
                        }
                        #generated_default
                    }
                },
                Unnamed(fields) => syn::Error::new_spanned(
                    fields,
                    "`#[freezable]` only supports structs with named fields",
                )
                .to_compile_error(),
                _unit => syn::Error::new_spanned(
                    &ident,
                    "`#[freezable]` only supports structs with named fields",
                )
                .to_compile_error(),
            }
        }
        _ => syn::Error::new_spanned(
            &ident,
            "`#[freezable]` can only be applied to structs with named fields",
        )
        .to_compile_error(),
    }.into()
}
