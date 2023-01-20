use crate::attr::{AttributeKind, RpcMethodAttribute};
use crate::to_gen_schema::generate_schema_method;
use crate::to_gen_schema::{MethodRegistration, RpcMethod};
use proc_macro::Ident;
use proc_macro2::Span;
use quote::quote;
use syn::{ItemTrait, ItemImpl};
use syn::{
    fold::{self, Fold},
    parse_quote, Result,
};

const METADATA_TYPE: &str = "Metadata";

const OPENRPC_SCHEMA_MODE_PREFIX: &str = "openrpc_schema_";

struct RpcTrait {
    methods: Vec<RpcMethod>,
    has_metadata: bool,
}

impl<'a> Fold for RpcTrait {
    fn fold_trait_item_method(&mut self, method: syn::TraitItemMethod) -> syn::TraitItemMethod {
        let mut foldable_method = method.clone();
        // strip rpc attributes
        foldable_method.attrs.retain(|a| {
            let rpc_method = self.methods.iter().find(|m| m.trait_item == method);
            rpc_method.map_or(true, |rpc| rpc.attr.attr != *a)
        });
        fold::fold_trait_item_method(self, foldable_method)
    }

    fn fold_trait_item_type(&mut self, ty: syn::TraitItemType) -> syn::TraitItemType {
        if ty.ident == METADATA_TYPE {
            self.has_metadata = true;
            let mut ty = ty.clone();
            ty.bounds.push(parse_quote!(_jsonrpc_core::Metadata));
            return ty;
        }
        ty
    }
}

fn compute_method_registrations(item_trait: &syn::ItemTrait) -> Result<Vec<MethodRegistration>> {
    let methods_result: Result<Vec<_>> = item_trait
        .items
        .iter()
        .filter_map(|trait_item| {
            if let syn::TraitItem::Method(method) = trait_item {
                match RpcMethodAttribute::parse_attr(method) {
                    Ok(Some(attr)) => Some(Ok(RpcMethod::new(attr, method.clone()))),
                    Ok(None) => None, // non rpc annotated trait method
                    Err(err) => Some(Err(syn::Error::new_spanned(method, err))),
                }
            } else {
                None
            }
        })
        .collect();
    let methods = methods_result?;

    let mut method_registrations: Vec<MethodRegistration> = Vec::new();

    for method in methods.iter() {
        match &method.attr().kind {
            AttributeKind::Rpc { has_metadata, .. } => {
                method_registrations.push(MethodRegistration::Standard {
                    method: method.clone(),
                    has_metadata: *has_metadata,
                })
            }
        }
    }
    Ok(method_registrations)
}

fn rpc_wrapper_mod_name(ident: &syn::Ident) -> syn::Ident {
    let name = ident.clone();
    let mod_name = format!("{}{}", OPENRPC_SCHEMA_MODE_PREFIX, name.to_string());
    syn::Ident::new(&mod_name, proc_macro2::Span::call_site())
}


fn handle_trait(mut rpc_trait: ItemTrait) -> Result<proc_macro2::TokenStream> {
    let method_registrations = compute_method_registrations(&rpc_trait)?;
    let mod_name_ident = rpc_wrapper_mod_name(&rpc_trait.ident);
    let generate_schema_method = generate_schema_method(&method_registrations)?;
    rpc_trait.items.push(parse_quote!(
        #[doc(hidden)]
        fn schema(&self) -> OpenrpcDocument;
    ));

    let openrpc_quote = quote!(

        use open_rpc_schema::document::OpenrpcDocument;

        pub mod #mod_name_ident {
            use super::*;
            use open_rpc_schema::document::*;
            #generate_schema_method
        }
        
        pub use self::#mod_name_ident::gen_schema;
        
        #rpc_trait
    );
    Ok(quote!(#openrpc_quote))
}


fn handle_impl(mut rpc_impl: ItemImpl)-> Result<proc_macro2::TokenStream>   {
    
    let r#trait = &rpc_impl.clone().trait_.unwrap().1;    
    let mod_name_ident = rpc_wrapper_mod_name(r#trait.get_ident().unwrap());
    let st = parse_quote!(
        fn schema(&self) -> OpenrpcDocument {
           super::#mod_name_ident::gen_schema()
        }
    );
    rpc_impl.items.push(st);
    Ok(quote!(#rpc_impl))
}

pub fn rpc_trait(input: syn::Item) -> Result<proc_macro2::TokenStream> {
    match input.clone() {
        syn::Item::Trait(item_trait) => handle_trait(item_trait),
        syn::Item::Impl(item_impl) => handle_impl(item_impl),
        item => {
            return Err(syn::Error::new_spanned(
                item,
                "The #[document_rpc] custom attribute only works with trait and impl declarations",
            ));
        }
    }
}
