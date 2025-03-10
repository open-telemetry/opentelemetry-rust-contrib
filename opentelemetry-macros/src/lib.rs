use proc_macro::TokenStream;

use syn::{parse_macro_input, ItemFn};

use crate::metrics::counted::CountedBuilder;

mod metrics;

#[proc_macro_attribute]
pub fn counted(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut builder = match CountedBuilder::try_from(attr, parse_macro_input!(item as ItemFn)) {
        Ok(value) => value,
        Err(err) => {
            return err;
        }
    };

    match builder.build() {
        Ok(value) => value,
        Err(err) => TokenStream::from(err.to_compile_error()),
    }
}
