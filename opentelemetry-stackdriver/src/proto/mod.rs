// TODO Remove below allow once proto files are fixed
#[allow(clippy::doc_markdown, clippy::doc_lazy_continuation)]
pub mod api;

pub mod devtools {
    pub mod cloudtrace {
        pub mod v2;
    }
}

pub mod logging {
    pub mod r#type;
    pub mod v2;
}

pub mod rpc;
