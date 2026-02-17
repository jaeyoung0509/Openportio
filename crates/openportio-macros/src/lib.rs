use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    parse::Parse, parse_macro_input, parse_quote, Attribute, Error, FnArg, GenericArgument, Ident,
    Item, ItemEnum, ItemFn, ItemStruct, LitStr, Pat, PatTupleStruct, PathArguments, PathSegment,
    Token, Type,
};

struct RouteArgs {
    method: RouteMethod,
    path: LitStr,
    auto_validate: bool,
    transparent: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RouteMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Clone, Copy)]
enum ExtractorKind {
    Json,
    Query,
    Path,
}

impl ExtractorKind {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "Json" => Some(Self::Json),
            "Query" => Some(Self::Query),
            "Path" => Some(Self::Path),
            _ => None,
        }
    }

    fn source_ident(self) -> &'static str {
        match self {
            Self::Json => "Json",
            Self::Query => "Query",
            Self::Path => "Path",
        }
    }

    fn validated_ident(self) -> &'static str {
        match self {
            Self::Json => "ValidatedJson",
            Self::Query => "ValidatedQuery",
            Self::Path => "ValidatedPath",
        }
    }
}

impl Parse for RouteArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let method_ident: Ident = input.parse()?;
        let method = match method_ident.to_string().as_str() {
            "get" => RouteMethod::Get,
            "post" => RouteMethod::Post,
            "put" => RouteMethod::Put,
            "patch" => RouteMethod::Patch,
            "delete" => RouteMethod::Delete,
            _ => {
                return Err(Error::new(
                    method_ident.span(),
                    "unsupported method; use one of: get, post, put, patch, delete",
                ))
            }
        };
        if input.is_empty() {
            return Err(Error::new(method_ident.span(), "route path is required"));
        }
        input.parse::<Token![,]>()?;

        let path: LitStr = input
            .parse()
            .map_err(|_| Error::new(input.span(), "route path must be a string literal"))?;

        let mut auto_validate = false;
        let mut transparent = false;
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            let flag: Ident = input.parse()?;
            match flag.to_string().as_str() {
                "auto_validate" => auto_validate = true,
                "transparent" => transparent = true,
                _ => {
                    return Err(Error::new(
                        flag.span(),
                        format!(
                            "unknown flag `{}`; supported flags are: auto_validate, transparent",
                            flag
                        ),
                    ))
                }
            }
        }

        if transparent && !auto_validate {
            return Err(Error::new(
                method_ident.span(),
                "`transparent` requires `auto_validate`; use `#[route(..., auto_validate, transparent)]`",
            ));
        }

        Ok(Self {
            method,
            path,
            auto_validate,
            transparent,
        })
    }
}

#[proc_macro_attribute]
pub fn route(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(args as RouteArgs);
    let mut item_fn = parse_macro_input!(item as ItemFn);

    if parsed.auto_validate {
        let server_crate = match resolve_openportio_server_path() {
            Ok(path) => path,
            Err(err) => return err.to_compile_error().into(),
        };
        let apply_result = if parsed.transparent {
            apply_transparent_auto_validate(&item_fn)
        } else {
            apply_auto_validate(&mut item_fn, &server_crate)
        };
        if let Err(err) = apply_result {
            return err.to_compile_error().into();
        }
    }

    let _ = (parsed.method, parsed.path);

    TokenStream::from(quote! { #item_fn })
}

#[proc_macro_attribute]
pub fn dto(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return Error::new(
            Span::call_site(),
            "`#[dto]` does not accept arguments; use it as `#[dto]`",
        )
        .to_compile_error()
        .into();
    }

    let mut item = parse_macro_input!(item as Item);
    let server_crate = match resolve_openportio_server_path() {
        Ok(path) => path,
        Err(err) => return err.to_compile_error().into(),
    };

    let apply_result = match &mut item {
        Item::Struct(ItemStruct { attrs, .. }) => ensure_dto_derives(attrs, &server_crate),
        Item::Enum(ItemEnum { attrs, .. }) => ensure_dto_derives(attrs, &server_crate),
        _ => Err(Error::new(
            item.span(),
            "`#[dto]` can only be used on structs or enums",
        )),
    };

    match apply_result {
        Ok(()) => TokenStream::from(quote!(#item)),
        Err(err) => err.to_compile_error().into(),
    }
}

fn resolve_openportio_server_path() -> syn::Result<syn::Path> {
    let found = crate_name("openportio-server").or_else(|_| crate_name("alloy-server"));
    match found {
        Ok(FoundCrate::Itself) => Ok(parse_quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let sanitized = name.replace('-', "_");
            let ident = Ident::new(&sanitized, Span::call_site());
            Ok(parse_quote!(::#ident))
        }
        Err(_) => Err(Error::new(
            Span::call_site(),
            "failed to resolve `openportio-server` crate for `#[route(..., auto_validate)]`; \
             ensure `openportio-server` (or legacy `alloy-server`) is present in Cargo.toml dependencies. \
             if you use the crate rename path (`openportio = { package = \"openportio-server\", ... }`), keep that dependency public to macro expansion",
        )),
    }
}

fn ensure_dto_derives(attrs: &mut Vec<Attribute>, server_crate: &syn::Path) -> syn::Result<()> {
    let required: [syn::Path; 3] = [
        parse_quote!(#server_crate::serde::Deserialize),
        parse_quote!(#server_crate::validator::Validate),
        parse_quote!(#server_crate::utoipa::ToSchema),
    ];
    let mut existing_last_segments = std::collections::BTreeSet::new();
    let mut first_derive: Option<(usize, Punctuated<syn::Path, Token![,]>)> = None;

    for (idx, attr) in attrs.iter().enumerate() {
        if !attr.path().is_ident("derive") {
            continue;
        }

        let derives = attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)?;
        for path in &derives {
            if let Some(last) = path.segments.last() {
                existing_last_segments.insert(last.ident.to_string());
            }
        }
        if first_derive.is_none() {
            first_derive = Some((idx, derives));
        }
    }

    let mut missing = Vec::new();
    for path in required {
        if let Some(last) = path.segments.last() {
            if !existing_last_segments.contains(&last.ident.to_string()) {
                missing.push(path);
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    if let Some((idx, mut derive_paths)) = first_derive {
        for path in missing {
            derive_paths.push(path);
        }
        attrs[idx] = parse_quote!(#[derive(#derive_paths)]);
    } else {
        attrs.insert(0, parse_quote!(#[derive(#(#missing),*)]));
    }

    Ok(())
}

fn apply_auto_validate(item_fn: &mut ItemFn, server_crate: &syn::Path) -> syn::Result<()> {
    let mut errors: Option<syn::Error> = None;

    for input in &mut item_fn.sig.inputs {
        if let FnArg::Typed(arg) = input {
            if let Err(err) = maybe_rewrite_typed_arg(arg, server_crate) {
                if let Some(existing) = &mut errors {
                    existing.combine(err);
                } else {
                    errors = Some(err);
                }
            }
        }
    }

    match errors {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn apply_transparent_auto_validate(item_fn: &ItemFn) -> syn::Result<()> {
    let mut errors: Option<syn::Error> = None;

    for input in &item_fn.sig.inputs {
        let FnArg::Typed(arg) = input else {
            continue;
        };
        let Some(segment) = last_type_segment(&arg.ty) else {
            continue;
        };

        let name = segment.ident.to_string();
        let Some(kind) = ExtractorKind::parse(&name) else {
            continue;
        };
        let validated = kind.validated_ident();
        let source = kind.source_ident();
        let message = format!(
            "transparent auto_validate mode does not rewrite extractors; replace `{source}<T>` with `{validated}<T>` in the handler signature, or remove `transparent` to keep legacy rewrite behavior"
        );

        let err = Error::new(segment.ident.span(), message);
        if let Some(existing) = &mut errors {
            existing.combine(err);
        } else {
            errors = Some(err);
        }
    }

    match errors {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn maybe_rewrite_typed_arg(arg: &mut syn::PatType, server_crate: &syn::Path) -> syn::Result<()> {
    let (kind, original_segment, inner_ty) = match extract_rewrite_target(&arg.ty)? {
        Some(values) => values,
        None => return Ok(()),
    };

    let validated_path = validated_extractor_path(server_crate, kind);
    let rewritten_ty: Type = parse_quote!(#validated_path<#inner_ty>);
    *arg.ty = rewritten_ty;

    rewrite_pattern(&mut arg.pat, kind, &validated_path, &original_segment)
}

fn extract_rewrite_target(ty: &Type) -> syn::Result<Option<(ExtractorKind, PathSegment, Type)>> {
    let Type::Path(type_path) = ty else {
        return Ok(None);
    };

    let Some(segment) = type_path.path.segments.last() else {
        return Ok(None);
    };

    let Some(kind) = ExtractorKind::parse(segment.ident.to_string().as_str()) else {
        return Ok(None);
    };

    let inner_ty = extract_single_generic_type(&segment.arguments).map_err(|err| {
        Error::new(
            segment.ident.span(),
            format!(
                "`{}` extractor in auto_validate must have exactly one type parameter: {err}",
                kind.source_ident()
            ),
        )
    })?;

    Ok(Some((kind, segment.clone(), inner_ty)))
}

fn last_type_segment(ty: &Type) -> Option<&PathSegment> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    type_path.path.segments.last()
}

fn rewrite_pattern(
    pat: &mut Box<Pat>,
    kind: ExtractorKind,
    validated_path: &syn::Path,
    original_segment: &PathSegment,
) -> syn::Result<()> {
    match pat.as_mut() {
        Pat::TupleStruct(PatTupleStruct { path, .. }) => {
            let Some(last) = path.segments.last() else {
                return Err(Error::new(
                    path.span(),
                    format!(
                        "unsupported `{}` pattern in auto_validate; use `{name}(value)` or `value: {name}<T>`. \
                         for explicit signatures without rewrite, use `#[route(..., auto_validate, transparent)]` with `{validated}<T>`",
                        kind.source_ident(),
                        name = kind.source_ident(),
                        validated = kind.validated_ident()
                    ),
                ));
            };

            let last_name = last.ident.to_string();
            let source = kind.source_ident();
            let validated = kind.validated_ident();
            if last_name != source && last_name != validated {
                return Err(Error::new(
                    last.ident.span(),
                    format!(
                        "pattern `{}` does not match extractor `{}` in auto_validate; expected `{}` pattern or switch to `transparent` mode with `{}`",
                        last_name,
                        source,
                        source,
                        kind.validated_ident()
                    ),
                ));
            }

            *path = validated_path.clone();
            Ok(())
        }
        Pat::Ident(ident_pat) => {
            if ident_pat.by_ref.is_some() || ident_pat.subpat.is_some() {
                return Err(Error::new(
                    ident_pat.span(),
                    format!(
                        "unsupported `{}` binding form in auto_validate; use simple binding like `value: {}<T>` \
                         or `transparent` mode with `{}`",
                        kind.source_ident(),
                        kind.source_ident(),
                        kind.validated_ident()
                    ),
                ));
            }

            let ident = ident_pat.ident.clone();
            let new_pat: Pat = if ident_pat.mutability.is_some() {
                parse_quote!(#validated_path(mut #ident))
            } else {
                parse_quote!(#validated_path(#ident))
            };
            **pat = new_pat;
            Ok(())
        }
        Pat::Wild(_) => {
            let new_pat: Pat = parse_quote!(#validated_path(_));
            **pat = new_pat;
            Ok(())
        }
        _ => Err(Error::new(
            pat.span(),
            format!(
                "unsupported pattern for `{}` in auto_validate; use `{}` destructuring (`{}(value)`) or simple binding (`value: {}<T>`). \
                 for no-rewrite signatures use `#[route(..., auto_validate, transparent)]` with `{}`",
                original_segment.ident,
                kind.source_ident(),
                kind.source_ident(),
                kind.source_ident(),
                kind.validated_ident(),
            ),
        )),
    }
}

fn validated_extractor_path(server_crate: &syn::Path, kind: ExtractorKind) -> syn::Path {
    match kind {
        ExtractorKind::Json => parse_quote!(#server_crate::api::ValidatedJson),
        ExtractorKind::Query => parse_quote!(#server_crate::api::ValidatedQuery),
        ExtractorKind::Path => parse_quote!(#server_crate::api::ValidatedPath),
    }
}

fn extract_single_generic_type(arguments: &PathArguments) -> syn::Result<Type> {
    let PathArguments::AngleBracketed(args) = arguments else {
        return Err(Error::new(Span::call_site(), "missing generic parameter"));
    };
    if args.args.len() != 1 {
        return Err(Error::new(
            Span::call_site(),
            "expected exactly one generic parameter",
        ));
    }
    match args.args.first() {
        Some(GenericArgument::Type(ty)) => Ok(ty.clone()),
        _ => Err(Error::new(
            Span::call_site(),
            "generic parameter must be a concrete type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_quote, parse_str};

    #[test]
    fn parses_method_path_and_auto_validate_flag() {
        let parsed = parse_str::<RouteArgs>(r#"post, "/notes", auto_validate"#)
            .expect("route args should parse");

        assert_eq!(parsed.method, RouteMethod::Post);
        assert_eq!(parsed.path.value(), "/notes");
        assert!(parsed.auto_validate);
        assert!(!parsed.transparent);
    }

    #[test]
    fn parses_without_auto_validate() {
        let parsed = parse_str::<RouteArgs>(r#"get, "/health""#).expect("route args should parse");

        assert_eq!(parsed.method, RouteMethod::Get);
        assert_eq!(parsed.path.value(), "/health");
        assert!(!parsed.auto_validate);
        assert!(!parsed.transparent);
    }

    #[test]
    fn parses_transparent_auto_validate_mode() {
        let parsed = parse_str::<RouteArgs>(r#"post, "/notes", auto_validate, transparent"#)
            .expect("route args should parse");

        assert_eq!(parsed.method, RouteMethod::Post);
        assert!(parsed.auto_validate);
        assert!(parsed.transparent);
    }

    #[test]
    fn rejects_transparent_without_auto_validate() {
        let err = match parse_str::<RouteArgs>(r#"post, "/notes", transparent"#) {
            Ok(_) => panic!("transparent without auto_validate must fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("requires `auto_validate`"));
    }

    #[test]
    fn rejects_unsupported_method() {
        let err = match parse_str::<RouteArgs>(r#"options, "/notes""#) {
            Ok(_) => panic!("unsupported method must fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("unsupported method"));
    }

    #[test]
    fn rejects_unknown_flag() {
        let err = match parse_str::<RouteArgs>(r#"post, "/notes", unknown_flag"#) {
            Ok(_) => panic!("unknown flag must fail"),
            Err(err) => err,
        };

        let message = err.to_string();
        assert!(message.contains("unknown flag"));
    }

    #[test]
    fn rejects_missing_path() {
        let err = match parse_str::<RouteArgs>("post") {
            Ok(_) => panic!("missing path must fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("path"));
    }

    #[test]
    fn rejects_non_string_path() {
        let err = match parse_str::<RouteArgs>("post, 10") {
            Ok(_) => panic!("non-string path must fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("string"));
    }

    #[test]
    fn auto_validate_rewrites_json_query_and_path_extractors() {
        let mut item_fn: ItemFn = parse_quote! {
            async fn create_note(
                Query(q): Query<ListQuery>,
                Json(body): Json<CreateNote>,
                Path(path): Path<NotePath>,
            ) {}
        };

        let server_crate: syn::Path = parse_quote!(::openportio_server);
        apply_auto_validate(&mut item_fn, &server_crate).expect("rewrite should work");

        let first = item_fn
            .sig
            .inputs
            .iter()
            .next()
            .expect("first arg should exist");
        let second = item_fn
            .sig
            .inputs
            .iter()
            .nth(1)
            .expect("second arg should exist");
        let third = item_fn
            .sig
            .inputs
            .iter()
            .nth(2)
            .expect("third arg should exist");

        assert_eq!(arg_type_ident(first), Some("ValidatedQuery".to_string()));
        assert_eq!(arg_pat_ident(first), Some("ValidatedQuery".to_string()));
        assert_eq!(arg_type_ident(second), Some("ValidatedJson".to_string()));
        assert_eq!(arg_pat_ident(second), Some("ValidatedJson".to_string()));
        assert_eq!(arg_type_ident(third), Some("ValidatedPath".to_string()));
        assert_eq!(arg_pat_ident(third), Some("ValidatedPath".to_string()));
    }

    #[test]
    fn auto_validate_rewrites_identifier_pattern_to_destructure() {
        let mut item_fn: ItemFn = parse_quote! {
            async fn create_note(query: Query<ListQuery>) {}
        };

        let server_crate: syn::Path = parse_quote!(::openportio_server);
        apply_auto_validate(&mut item_fn, &server_crate).expect("rewrite should work");

        let first = item_fn.sig.inputs.iter().next().expect("arg should exist");
        assert_eq!(arg_type_ident(first), Some("ValidatedQuery".to_string()));
        assert_eq!(arg_pat_ident(first), Some("ValidatedQuery".to_string()));
    }

    #[test]
    fn auto_validate_reports_actionable_error_for_unsupported_pattern() {
        let mut item_fn: ItemFn = parse_quote! {
            async fn create_note((query): Query<ListQuery>) {}
        };

        let server_crate: syn::Path = parse_quote!(::openportio_server);
        let err = apply_auto_validate(&mut item_fn, &server_crate).expect_err("must fail");
        assert!(err.to_string().contains("unsupported pattern"));
    }

    #[test]
    fn transparent_auto_validate_rejects_hidden_rewrite_extractors() {
        let item_fn: ItemFn = parse_quote! {
            async fn create_note(Query(q): Query<ListQuery>, Json(body): Json<CreateNote>) {}
        };

        let err = apply_transparent_auto_validate(&item_fn).expect_err("must fail");
        let message = err.to_string();
        assert!(message.contains("does not rewrite extractors"));
        assert!(message.contains("ValidatedQuery"), "message: {message}");
    }

    #[test]
    fn transparent_auto_validate_accepts_explicit_validated_extractors() {
        let item_fn: ItemFn = parse_quote! {
            async fn create_note(
                ::openportio_server::api::ValidatedQuery(query): ::openportio_server::api::ValidatedQuery<ListQuery>,
                ::openportio_server::api::ValidatedJson(body): ::openportio_server::api::ValidatedJson<CreateNote>
            ) {}
        };

        apply_transparent_auto_validate(&item_fn)
            .expect("explicit validated extractors should pass");
    }

    #[test]
    fn without_auto_validate_keeps_original_extractors() {
        let mut item_fn: ItemFn = parse_quote! {
            async fn create_note(Query(q): Query<ListQuery>, Json(body): Json<CreateNote>) {}
        };
        let args = parse_str::<RouteArgs>(r#"post, "/notes""#).expect("route args should parse");

        if args.auto_validate {
            let server_crate: syn::Path = parse_quote!(::openportio_server);
            apply_auto_validate(&mut item_fn, &server_crate).expect("rewrite should work");
        }

        let rendered = quote!(#item_fn).to_string();
        assert!(rendered.contains("Query"));
        assert!(rendered.contains("Json"));
        assert!(!rendered.contains("ValidatedQuery"));
        assert!(!rendered.contains("ValidatedJson"));
    }

    #[test]
    fn resolved_path_uses_callsite_crate_alias() {
        let path: syn::Path = match FoundCrate::Name("meld_api".to_string()) {
            FoundCrate::Name(name) => {
                let ident = Ident::new(&name.replace('-', "_"), Span::call_site());
                parse_quote!(::#ident)
            }
            FoundCrate::Itself => parse_quote!(crate),
        };

        let rendered = quote!(#path).to_string();
        assert_eq!(rendered, ":: meld_api");
    }

    #[test]
    fn dto_injects_deserialize_validate_and_schema_derives() {
        let mut item: ItemStruct = parse_quote! {
            struct Payload {
                #[validate(length(min = 1))]
                name: String,
            }
        };
        let server_crate: syn::Path = parse_quote!(::openportio_server);
        ensure_dto_derives(&mut item.attrs, &server_crate).expect("dto derives should be injected");

        let rendered = quote!(#item).to_string();
        assert!(rendered.contains(":: openportio_server :: serde :: Deserialize"));
        assert!(rendered.contains(":: openportio_server :: validator :: Validate"));
        assert!(rendered.contains(":: openportio_server :: utoipa :: ToSchema"));
    }

    #[test]
    fn dto_keeps_existing_derive_and_appends_missing() {
        let mut item: ItemStruct = parse_quote! {
            #[derive(Debug, serde::Deserialize)]
            struct Payload {
                #[validate(length(min = 1))]
                name: String,
            }
        };
        let server_crate: syn::Path = parse_quote!(::openportio_server);
        ensure_dto_derives(&mut item.attrs, &server_crate).expect("dto derives should be injected");

        let rendered = quote!(#item).to_string();
        assert!(rendered.contains("Debug"));
        assert!(rendered.contains("serde :: Deserialize"));
        assert!(rendered.contains(":: openportio_server :: validator :: Validate"));
        assert!(rendered.contains(":: openportio_server :: utoipa :: ToSchema"));
    }

    fn arg_type_ident(arg: &FnArg) -> Option<String> {
        let FnArg::Typed(arg) = arg else {
            return None;
        };
        let Type::Path(type_path) = arg.ty.as_ref() else {
            return None;
        };
        type_path.path.segments.last().map(|s| s.ident.to_string())
    }

    fn arg_pat_ident(arg: &FnArg) -> Option<String> {
        let FnArg::Typed(arg) = arg else {
            return None;
        };
        let Pat::TupleStruct(tuple_struct) = arg.pat.as_ref() else {
            return None;
        };
        tuple_struct
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
    }
}
