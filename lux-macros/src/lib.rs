use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Lit, Type};

/// Derive macro that generates a default implementation of `DisplayAsLuaKV` for structs.
///
/// # Required struct-level attribute
/// ```ignore
/// #[display_lua(key = "some_key")]
/// ```
/// # Field-level attributes (optional)
/// - `#[display_lua(rename = "other_name")]` - use a different Lua key for this field
/// - `#[display_lua(convert_with = "func_name")]` - use a custom conversion function for this field
///
/// # Special Type Treatment
/// If the type is an `Option`, `Vec` or `HashMap`, no entry will be generated if the value is
/// `None` or empty.
#[proc_macro_derive(DisplayAsLuaKV, attributes(display_lua))]
pub fn derive_display_as_lua_kv(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match impl_display_as_lua_kv(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn impl_display_as_lua_kv(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let outer_key = parse_struct_key_attr(input)?;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "DisplayAsLuaKV only supports structs with named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "DisplayAsLuaKV can only be derived for structs",
            ))
        }
    };

    let field_stmts: Vec<TokenStream2> = fields
        .iter()
        .map(generate_field_stmt)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl #impl_generics crate::lua_rockspec::DisplayAsLuaKV for #struct_name #ty_generics #where_clause {
            fn display_lua(&self) -> crate::lua_rockspec::DisplayLuaKV {
                let mut __fields: Vec<crate::lua_rockspec::DisplayLuaKV> = Vec::new();
                #(#field_stmts)*
                crate::lua_rockspec::DisplayLuaKV {
                    key: #outer_key.to_string(),
                    value: crate::lua_rockspec::DisplayLuaValue::Table(__fields),
                }
            }
        }
    })
}

/// Parses `#[display_lua(key = "some_key")]` from the struct's attributes.
fn parse_struct_key_attr(input: &DeriveInput) -> syn::Result<String> {
    for attr in &input.attrs {
        if !attr.path().is_ident("display_lua") {
            continue;
        }
        let mut found_key: Option<String> = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    found_key = Some(s.value());
                    Ok(())
                } else {
                    Err(meta.error("expected a string literal for `key`"))
                }
            } else {
                Err(meta.error("unknown attribute: expected `key = \"...\"`"))
            }
        })?;
        if let Some(k) = found_key {
            return Ok(k);
        }
    }
    Err(syn::Error::new_spanned(
        input,
        "DisplayAsLuaKV requires `#[display_lua(key = \"...\")]` on the struct",
    ))
}

/// Per-field parsed attributes.
#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    convert_with: Option<syn::Path>,
}

fn parse_field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("display_lua") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    attrs.rename = Some(s.value());
                    Ok(())
                } else {
                    Err(meta.error("expected a string literal for `rename`"))
                }
            } else if meta.path.is_ident("convert_with") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    attrs.convert_with = Some(s.parse_with(syn::Path::parse_mod_style)?);
                    Ok(())
                } else {
                    Err(meta.error("expected a string literal for `convert_with`"))
                }
            } else {
                Err(meta.error(
                    "unknown display_lua field attribute; expected `rename` or `convert_with`",
                ))
            }
        })?;
    }
    Ok(attrs)
}

/// Classifies the outermost wrapper of a type (we use it for custom handling of types)
enum TypeKind {
    Option,
    Vec,
    HashMap,
    Plain,
}

fn classify_type(ty: &Type) -> TypeKind {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let name = segment.ident.to_string();
            match name.as_str() {
                "Option" => return TypeKind::Option,
                "Vec" => return TypeKind::Vec,
                "HashMap" | "BTreeMap" | "IndexMap" => return TypeKind::HashMap,
                _ => {}
            }
        }
    }
    TypeKind::Plain
}

fn generate_field_stmt(field: &syn::Field) -> syn::Result<TokenStream2> {
    let ident = field.ident.as_ref().expect("named field");
    let attrs = parse_field_attrs(field)?;

    let lua_key = attrs
        .rename
        .as_deref()
        .map(String::from)
        .unwrap_or_else(|| ident.to_string());
    let kind = classify_type(&field.ty);

    let stmt = match kind {
        TypeKind::Option => {
            let value_expr = if let Some(func) = &attrs.convert_with {
                quote! { #func(__inner) }
            } else {
                quote! {
                    crate::lua_rockspec::DisplayAsLuaValue::display_lua_value(__inner)
                }
            };
            quote! {
                if let Some(__inner) = &self.#ident {
                    __fields.push(crate::lua_rockspec::DisplayLuaKV {
                        key: #lua_key.to_string(),
                        value: #value_expr,
                    });
                }
            }
        }
        TypeKind::Vec | TypeKind::HashMap => {
            let value_expr = if let Some(func) = &attrs.convert_with {
                quote! { #func(&self.#ident) }
            } else {
                quote! {
                    crate::lua_rockspec::DisplayAsLuaValue::display_lua_value(&self.#ident)
                }
            };
            quote! {
                if !self.#ident.is_empty() {
                    __fields.push(crate::lua_rockspec::DisplayLuaKV {
                        key: #lua_key.to_string(),
                        value: #value_expr,
                    });
                }
            }
        }
        TypeKind::Plain => {
            let value_expr = if let Some(func) = &attrs.convert_with {
                quote! { #func(&self.#ident) }
            } else {
                quote! {
                    crate::lua_rockspec::DisplayAsLuaValue::display_lua_value(&self.#ident)
                }
            };
            quote! {
                __fields.push(crate::lua_rockspec::DisplayLuaKV {
                    key: #lua_key.to_string(),
                    value: #value_expr,
                });
            }
        }
    };

    Ok(stmt)
}
