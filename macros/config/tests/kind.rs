use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::{Data, DeriveInput, Error, Fields, Ident, LitStr, Result, Type};

pub fn derive(input: TokenStream) -> Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let name = &input.ident;

    let ident = kind_ident(&input)?;
    let fields = KindField::vec_from_input(&input)?;

    let shadow = format_ident!("__{}Shadow", name);
    let shadow_fields = fields.iter().map(|f| {
        let (n, ty) = (&f.name, &f.ty);
        quote!(#n: #ty)
    });
    let shadow_masks = fields.iter().filter_map(|f| {
        let key = Ident::new(f.ignore_key.as_ref()?, Span::call_site());
        Some(quote!(#key: ::std::option::Option<bool>))
    });

    let collapse = fields.iter().map(|f| {
        let n = &f.name;
        let Some(key) = &f.ignore_key else {
            return quote!(#n: shadow.#n);
        };
        let mask = Ident::new(key, Span::call_site());
        quote! {
            #n: {
                if let Some(false) = shadow.#mask {
                    return Err(D::Error::custom(concat!(
                        "`", #key, "` may only be set to true; write the value \
                         itself to stop ignoring it"
                    )));
                }
                if shadow.#mask.is_some() && shadow.#n.is_some() {
                    return Err(D::Error::custom(concat!(
                        "`", stringify!(#n), "` and `", #key,
                        "` are mutually exclusive"
                    )));
                }
                shadow.#n
            }
        }
    });

    let field_attrs = fields.iter().map(|f| {
        let name = LitStr::new(&f.name.to_string(), Span::call_site());
        let ignore_key = match &f.ignore_key {
            Some(k) => quote!(Some(#k)),
            None => quote!(None),
        };
        let (is_path, deep_merge) = (f.is_relpath, f.deep_merge);
        let clears: Vec<LitStr> = f
            .clears
            .iter()
            .map(|c| LitStr::new(&c.to_string(), Span::call_site()))
            .collect();
        quote! {
            crate::config::tests::kind::FieldAttrs {
                name: #name,
                ignore_key: #ignore_key,
                is_relpath: #is_path,
                clears: &[#(#clears),*],
                deep_merge: #deep_merge,
            }
        }
    });

    let apply_arms = fields.iter().map(|f| f.apply_arm());
    let clear_passes = fields.iter().filter(|f| !f.clears.is_empty()).map(|f| {
        let key = LitStr::new(&f.name.to_string(), Span::call_site());
        let resets = f.clears.iter().map(|c| {
            let cleared = LitStr::new(&c.to_string(), Span::call_site());
            quote! {
                if !overrides.contains_key(#cleared) {
                    self.#c = Default::default();
                }
            }
        });
        quote! {
            if overrides.contains_key(#key) {
                #(#resets)*
            }
        }
    });

    Ok(quote! {
        impl crate::config::tests::kind::TestKind for #name {
            const IDENT: &'static str = #ident;

            const FIELDS: &'static [crate::config::tests::kind::FieldAttrs] =
                &[#(#field_attrs),*];

            fn apply(
                &mut self,
                overrides: &toml::Table,
                ctx: &crate::config::tests::kind::ApplyCtx,
            ) -> Result<(), crate::error::Error> {
                let _ = ctx;
                // Discard inherited values that this file's own keys invalidate,
                // before applying anything: a field the file sets itself is kept,
                // and the order keys happen to appear in must not matter.
                #(#clear_passes)*
                for (key, value) in overrides {
                    match key.as_str() {
                        #(#apply_arms)*
                        _ => {
                            return Err(crate::error::Error::test_config_msg(
                                "invalid test option key",
                            )
                            .key(key.as_str())
                            .kind(#ident)
                            .into())
                        }
                    }
                }
                Ok(())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                use ::serde::de::Error as _;

                #[derive(::serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct #shadow {
                    #(#shadow_fields,)*
                    #(#shadow_masks,)*
                }

                let shadow = #shadow::deserialize(deserializer)?;
                Ok(Self { #(#collapse,)* })
            }
        }
    })
}

/// Reads `#[testkind(ident = "...")]` off the struct.
fn kind_ident(input: &DeriveInput) -> Result<LitStr> {
    let mut ident: Option<LitStr> = None;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("testkind")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("ident") {
                ident = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error(format!(
                    "unknown `testkind` option `{}`, expected `ident`",
                    meta.path.to_token_stream()
                )))
            }
        })?;
    }
    ident.ok_or_else(|| Error::new_spanned(&input.ident, "missing `#[testkind(ident = \"...\")]`"))
}

/// One field of a test kind options struct.
struct KindField {
    name: Ident,
    ty: Type,

    /// Key that masks this field, for `#[testkind(ignorable)]` fields. Defaults
    /// to the field name with an `_ignore` suffix.
    ignore_key: Option<String>,

    /// `#[testkind(relpath)]`: resolve the value against the directory of the file
    /// that wrote it.
    is_relpath: bool,

    /// `#[testkind(clears(a, b))]`: reset these fields whenever this one is set.
    clears: Vec<Ident>,

    /// `#[testkind(merge = "deep")]`: merge a table key-by-key rather than
    /// replacing it wholesale.
    deep_merge: bool,
}

impl KindField {
    /// Reads the fields and their `#[testkind(...)]` attributes.
    fn vec_from_input(input: &DeriveInput) -> Result<Vec<KindField>> {
        let Data::Struct(data) = &input.data else {
            return Err(Error::new_spanned(
                &input.ident,
                "TestKind can only be derived for structs",
            ));
        };
        let Fields::Named(named) = &data.fields else {
            return Err(Error::new_spanned(
                &input.ident,
                "TestKind requires named fields",
            ));
        };

        named
            .named
            .iter()
            .map(|f| {
                let name = f.ident.clone().expect("named field");
                let mut field = KindField {
                    ty: f.ty.clone(),
                    ignore_key: None,
                    is_relpath: false,
                    clears: vec![],
                    deep_merge: false,
                    name,
                };

                for attr in f.attrs.iter().filter(|a| a.path().is_ident("testkind")) {
                    attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("ignorable") {
                            field.ignore_key = Some(format!("{}_ignore", field.name));
                            // `ignorable(key = "...")` overrides the default key.
                            if meta.input.peek(syn::token::Paren) {
                                meta.parse_nested_meta(|inner| {
                                    if inner.path.is_ident("key") {
                                        field.ignore_key =
                                            Some(inner.value()?.parse::<LitStr>()?.value());
                                        Ok(())
                                    } else {
                                        Err(inner.error("expected `key = \"...\"`"))
                                    }
                                })?;
                            }
                        } else if meta.path.is_ident("relpath") {
                            field.is_relpath = true;
                        } else if meta.path.is_ident("clears") {
                            meta.parse_nested_meta(|inner| {
                                field.clears.push(
                                    inner
                                        .path
                                        .get_ident()
                                        .cloned()
                                        .ok_or_else(|| inner.error("expected a field name"))?,
                                );
                                Ok(())
                            })?;
                        } else if meta.path.is_ident("merge") {
                            let how: LitStr = meta.value()?.parse()?;
                            match how.value().as_str() {
                                "deep" => field.deep_merge = true,
                                _ => return Err(Error::new_spanned(&how, "expected `\"deep\"`")),
                            }
                        } else {
                            return Err(meta.error(format!(
                                "unknown `testkind` option `{}`, expected one of \
                             `ignorable`, `path`, `clears`, `merge`",
                                meta.path.to_token_stream()
                            )));
                        }
                        Ok(())
                    })?;
                }

                Ok(field)
            })
            .collect()
    }

    /// The match arm applying one option key for a field kind.
    fn apply_arm(&self) -> TokenStream {
        let n = &self.name;
        let key = LitStr::new(&n.to_string(), Span::call_site());

        let convert = quote! {
            value.to_owned().try_into().map_err(|e| {
                crate::error::Error::test_config_msg("invalid value for test option")
                    .key(#key)
                    .as_error()
                    .with_cause(Box::new(crate::error::Error::from(e)))
            })?
        };

        let set = if self.deep_merge {
            quote! {
                let table = value.as_table().ok_or_else(|| {
                    crate::error::Error::test_config_msg("expected a table").key(#key)
                })?;
                for (k, v) in table {
                    let v = v.to_owned().try_into().map_err(|e| {
                        crate::error::Error::test_config_msg("invalid value for test option")
                            .key(#key)
                            .as_error()
                            .with_cause(Box::new(crate::error::Error::from(e)))
                    })?;
                    self.#n.insert(k.to_owned(), v);
                }
            }
        } else if self.is_relpath {
            let resolved = quote! {
                {
                    let raw: String = #convert;
                    crate::utils::path_absolute_join(ctx.dir, raw)?
                }
            };
            match self.ignore_key {
                Some(_) => quote!(self.#n = Some(#resolved);),
                None => quote!(self.#n = #resolved;),
            }
        } else if self.ignore_key.is_some() {
            quote!(self.#n = Some(#convert);)
        } else {
            quote!(self.#n = #convert;)
        };

        let main = quote! {
            #key => { #set }
        };

        let Some(ignore_key) = &self.ignore_key else {
            return main;
        };
        let ignore_lit = LitStr::new(ignore_key, Span::call_site());
        quote! {
            #main
            #ignore_lit => match value.as_bool() {
                Some(true) => self.#n = None,
                Some(false) => {
                    return Err(crate::error::Error::test_config_msg(
                        "may only be set to true; write the value itself to stop ignoring it",
                    )
                    .key(#ignore_lit)
                    .into())
                }
                None => {
                    return Err(crate::error::Error::test_config_msg("expected a boolean")
                        .key(#ignore_lit)
                        .into())
                }
            },
        }
    }
}
