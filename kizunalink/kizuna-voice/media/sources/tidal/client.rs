// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use std::sync::Arc;

use serde_json::Value;
use tracing::debug;

use super::error::{TidalError, TidalResult};

pub struct HifiClient {
    pub inner: Arc<reqwest::Client>,
    pub base_urls: Vec<String>,
    pub quality_order: Vec<String>,
    pub country_code: String,
}

impl HifiClient {
    pub fn new(
        inner: Arc<reqwest::Client>,
        mut base_urls: Vec<String>,
        quality_order: Vec<String>,
        country_code: String,
    ) -> Result<Self, String> {
        if base_urls.is_empty() {
            return Err(
                "hifi_apis must contain at least one HiFi API URL (e.g. http://localhost:8000)"
                    .to_string(),
            );
        }
        if quality_order.is_empty() {
            return Err("hifi_qualities must not be empty".to_string());
        }
        for url in &mut base_urls {
            while url.ends_with('/') {
                url.pop();
            }
        }
        Ok(Self { inner, base_urls, quality_order, country_code })
    }

    pub async fn get(&self, path: &str, params: &[(&str, &str)]) -> TidalResult<Value> {
        let mut last_err = TidalError::AllApisFailed;

        for base_url in &self.base_urls {
            let url = format!("{}{}", base_url, path);
            debug!("HiFi: GET {} params={:?}", url, params);

            let resp = match self.inner.get(&url).query(params).send().await {
                Ok(r) => r,
                Err(e) => {
                    debug!("HiFi: {} unreachable — {}", base_url, e);
                    last_err = TidalError::Request(e);
                    continue;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                debug!("HiFi: {} → {} — {}", url, status, &body);
                last_err = TidalError::ApiError { status, body };
                continue;
            }

            return resp.json::<Value>().await.map_err(TidalError::Request);
        }

        Err(last_err)
    }
}