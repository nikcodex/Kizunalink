// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod client;
pub mod http;
pub(crate) mod range;
pub mod segmented;
pub mod traits;

pub use client::{create_client, create_client_with_pinning};
pub use http::HttpSource;
pub use segmented::SegmentedSource;
pub use traits::AudioSource;
