use proc_macro::TokenStream;
use std::sync::{Mutex, OnceLock};

use syn::__private::ToTokens;
use syn::meta::ParseNestedMeta;
use syn::parse::Parser;
use syn::{Block, ItemFn, LitBool, LitStr};

pub const EXPORTER_CRATE: &'static str = "opentelemetry_contrib";

pub struct CountedBuilder {
    item_fn: Option<ItemFn>,
    attrs: CountedAttributes,
}

impl CountedBuilder {
    pub fn try_from(attr: TokenStream, item_fn: ItemFn) -> Result<CountedBuilder, TokenStream> {
        let attrs = CountedAttributes::try_from(attr, &item_fn)?;
        Ok(Self {
            item_fn: Some(item_fn),
            attrs,
        })
    }

    fn check_metric_name_availability(&self) {
        const DETECTED_METRIC_NAME_DUPLICATION: &'static str = "detected metric name duplication!";
        const LOCK_ERROR_METRIC_NAME_CHECKER: &'static str = "unexpected error encountered trying to lock shared data structure for metric name availability checker!";

        static METRIC_NAMES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
        let metric_names_value = METRIC_NAMES.get_or_init(|| Mutex::new(Vec::new()));

        let attrs = &self.attrs;
        match metric_names_value.lock() {
            Ok(mut names) => {
                if names.contains(&attrs.name.as_str().to_string()) {
                    panic!("{}", DETECTED_METRIC_NAME_DUPLICATION);
                }
                names.push(attrs.name.clone())
            }
            Err(_) => {
                panic!("{}", LOCK_ERROR_METRIC_NAME_CHECKER);
            }
        }
    }

    fn build_code_block(&self) -> Result<Block, syn::Error> {
        let label_size = &self.attrs.labels_size;
        let labels = &self.attrs.labels;
        let meter_provider = &self.attrs.meter_provider;
        let name = &self.attrs.name;
        let description = &self.attrs.description;

        let result = syn::parse_str::<Block>(&format!(
            r#"
            {{
        static LABELS: std::sync::OnceLock<[{EXPORTER_CRATE}::opentelemetry::KeyValue; {label_size}]> = std::sync::OnceLock::new();
        let labels_value = LABELS.get_or_init(|| {{
            [{labels}]
        }});

        static COUNTER: std::sync::OnceLock<{EXPORTER_CRATE}::opentelemetry::metrics::Counter<u64>> = std::sync::OnceLock::new();
        let counter_value = COUNTER.get_or_init(|| {{
                let meter = {EXPORTER_CRATE}::opentelemetry::global::meter("{meter_provider}");
                meter.u64_counter("{name}").with_description("{description}").init()
        }});

        counter_value.add(1, labels_value);
            }}
    "#
        ))?;

        Ok(result)
    }

    pub fn build(&mut self) -> Result<TokenStream, syn::Error> {
        let mut item_fn = self.item_fn.take().unwrap();
        if !self.attrs.enabled {
            return Ok(TokenStream::from(item_fn.into_token_stream()));
        }

        self.check_metric_name_availability();
        let mut code_block = self.build_code_block()?;

        code_block.stmts.extend_from_slice(&*item_fn.block.stmts);
        item_fn.block.stmts = code_block.stmts;

        Ok(TokenStream::from(item_fn.into_token_stream()))
    }
}

pub struct CountedAttributes {
    name: String,
    description: String,
    meter_provider: String,
    enabled: bool,
    labels: String,
    labels_size: u8,
}

impl CountedAttributes {
    pub fn try_from(attr: TokenStream, item_fn: &ItemFn) -> Result<Self, TokenStream> {
        const DEFAULT_METER_PROVIDER_NAME: &'static str = "default_meter_provider";
        const DEFAULT_DESCRIPTION: &'static str = "Empty description!";
        const METER_PROVIDER_NAME_ATTR_NAME: &'static str = "meter_provider";
        const NAME_ATTR_NAME: &'static str = "name";
        const ENABLED_ATTR_NAME: &'static str = "enabled";
        const DESCRIPTION_ATTR_NAME: &'static str = "description";
        const LABELS_ATTR_NAME: &'static str = "labels";
        const ATTR_ERROR_MESSAGE: &'static str = "unsupported attribute for counted macro!";

        let mut name = format!("fn_{}_count", item_fn.sig.ident.to_string());
        let mut enabled = true;
        let mut meter_provider = DEFAULT_METER_PROVIDER_NAME.to_string();
        let mut description = DEFAULT_DESCRIPTION.to_string();
        let mut labels = "".to_string();
        let mut labels_size = 0;
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident(NAME_ATTR_NAME) {
                name = meta.value()?.parse::<LitStr>()?.value();
            } else if meta.path.is_ident(ENABLED_ATTR_NAME) {
                enabled = meta.value()?.parse::<LitBool>()?.value();
            } else if meta.path.is_ident(METER_PROVIDER_NAME_ATTR_NAME) {
                meter_provider = meta.value()?.parse::<LitStr>()?.value();
            } else if meta.path.is_ident(DESCRIPTION_ATTR_NAME) {
                description = meta.value()?.parse::<LitStr>()?.value();
            } else if meta.path.is_ident(LABELS_ATTR_NAME) {
                let (labels_res, labels_size_res) = Self::process_labels_attr(&meta)?;
                labels = labels_res;
                labels_size = labels_size_res;
            } else {
                return Err(meta.error(ATTR_ERROR_MESSAGE));
            }
            Ok(())
        });

        if let Err(err) = parser.parse(attr) {
            return Err(TokenStream::from(err.to_compile_error()));
        }

        Ok(Self {
            name,
            description,
            meter_provider,
            enabled,
            labels,
            labels_size,
        })
    }

    fn process_labels_attr(meta: &ParseNestedMeta) -> Result<(String, u8), syn::Error> {
        const LABELS_LENGTH_ERROR_MESSAGE: &'static str =
            "invalid arguments provided in labels attribute! (must be provided list of key-value)";

        let lebels_as_str = meta.value()?.parse::<LitStr>()?.value();
        let labels_as_array: Vec<String> = lebels_as_str
            .split(",")
            .into_iter()
            .map(|v| v.trim().to_string())
            .collect();

        if labels_as_array.len() % 2 != 0 {
            panic!("{}", LABELS_LENGTH_ERROR_MESSAGE);
        }

        let labels_size = labels_as_array.len() as u8 / 2;
        let mut labels = "".to_string();
        for label_chunk in labels_as_array.chunks(2) {
            if let [key, value] = label_chunk {
                if !labels.is_empty() {
                    labels.push_str(", ");
                }
                labels.push_str(&format!(
                    r#"{EXPORTER_CRATE}::opentelemetry::KeyValue::new("{key}", "{value}")"#
                ));
            }
        }

        Ok((labels, labels_size))
    }
}
