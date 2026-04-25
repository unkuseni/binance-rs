use error_chain::bail;
use hex::encode as hex_encode;
use hmac::{Hmac, Mac};
use crate::errors::{BinanceContentError, ErrorKind, Result};
use reqwest::StatusCode;
use reqwest::Response;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT, CONTENT_TYPE};
use sha2::Sha256;
use serde::de::DeserializeOwned;
use crate::api::API;

#[derive(Clone)]
pub struct Client {
    api_key: String,
    secret_key: String,
    host: String,
    inner_client: reqwest::Client,
    verbose: bool,

    api_key_redacted: String,
}

impl Client {
    pub fn new(api_key: Option<String>, secret_key: Option<String>, host: String) -> Self {
        let api_key = api_key.unwrap_or_default();
        let api_key_redacted = if api_key.len() > 8 {
            format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
        } else if !api_key.is_empty() {
            "***".into()
        } else {
            String::new()
        };
        Client {
            api_key,
            secret_key: secret_key.unwrap_or_default(),
            host,
            inner_client: reqwest::Client::builder()
                .pool_idle_timeout(None)
                .build()
                .unwrap(),
            verbose: false,
            api_key_redacted,
        }
    }

    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    pub fn set_host(&mut self, host: String) {
        self.host = host;
    }

    pub async fn get_signed<T: DeserializeOwned>(
        &self, endpoint: API, request: Option<String>,
    ) -> Result<T> {
        let url = self.sign_request(endpoint, request)?;
        let headers = self.build_headers(true)?;
        if self.verbose {
            println!("Request URL: {}", url);
        }
        let client = &self.inner_client;
        let response = client.get(url.as_str()).headers(headers).send().await?;

        self.handler(response).await
    }

    pub async fn post_signed<T: DeserializeOwned>(
        &self, endpoint: API, request: String,
    ) -> Result<T> {
        let url = self.sign_request(endpoint, Some(request))?;
        let client = &self.inner_client;

        let headers = self.build_headers(true)?;
        if self.verbose {
            println!("Request URL: {}", url);
        }
        let response = client.post(url.as_str()).headers(headers).send().await?;

        self.handler(response).await
    }

    pub async fn delete_signed<T: DeserializeOwned>(
        &self, endpoint: API, request: Option<String>,
    ) -> Result<T> {
        let url = self.sign_request(endpoint, request)?;
        let headers = self.build_headers(true)?;
        if self.verbose {
            println!("Request URL: {}", url);
        }
        let client = &self.inner_client;
        let response = client.delete(url.as_str()).headers(headers).send().await?;

        self.handler(response).await
    }

    pub async fn get<T: DeserializeOwned>(
        &self, endpoint: API, request: Option<String>,
    ) -> Result<T> {
        let mut url: String = format!("{}{}", self.host, String::from(endpoint));
        if let Some(request) = request {
            if !request.is_empty() {
                url.push_str(format!("?{}", request).as_str());
            }
        }

        let client = &self.inner_client;
        if self.verbose {
            println!("Request URL: {}", url);
        }
        let response = client.get(url.as_str()).send().await?;

        self.handler(response).await
    }

    pub async fn post<T: DeserializeOwned>(&self, endpoint: API) -> Result<T> {
        let url: String = format!("{}{}", self.host, String::from(endpoint));

        let client = &self.inner_client;
        let response = client
            .post(url.as_str())
            .headers(self.build_headers(false)?)
            .send()
            .await?;

        self.handler(response).await
    }

    pub async fn put<T: DeserializeOwned>(&self, endpoint: API, listen_key: &str) -> Result<T> {
        let url: String = format!("{}{}", self.host, String::from(endpoint));
        let data: String = format!("listenKey={}", listen_key);

        let client = &self.inner_client;

        let headers = self.build_headers(true)?;
        if self.verbose {
            println!("Request URL: {}", url);
            println!("Request Body: {}", data);
        }
        let response = client
            .put(url.as_str())
            .headers(headers)
            .body(data)
            .send()
            .await?;

        self.handler(response).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, endpoint: API, listen_key: &str) -> Result<T> {
        let url: String = format!("{}{}", self.host, String::from(endpoint));
        let data: String = format!("listenKey={}", listen_key);

        let client = &self.inner_client;
        let response = client
            .delete(url.as_str())
            .headers(self.build_headers(false)?)
            .body(data)
            .send()
            .await?;

        self.handler(response).await
    }

    // Request must be signed
    fn sign_request(&self, endpoint: API, request: Option<String>) -> Result<String> {
        let mut signed_key = match Hmac::<Sha256>::new_from_slice(self.secret_key.as_bytes()) {
            Ok(key) => key,
            Err(_) => bail!("Invalid secret key"),
        };
        if let Some(request) = request {
            signed_key.update(request.as_bytes());
            let signature = hex_encode(signed_key.finalize().into_bytes());
            let request_body: String = format!("{}&signature={}", request, signature);
            Ok(format!("{}{}?{}", self.host, String::from(endpoint), request_body))
        } else {
            let signature = hex_encode(signed_key.finalize().into_bytes());
            let request_body: String = format!("&signature={}", signature);
            Ok(format!("{}{}?{}", self.host, String::from(endpoint), request_body))
        }
    }

    fn build_headers(&self, content_type: bool) -> Result<HeaderMap> {
        let mut custom_headers = HeaderMap::new();

        custom_headers.insert(USER_AGENT, HeaderValue::from_static("binance-rs"));
        if content_type {
            custom_headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            );
        }
        custom_headers.insert(
            HeaderName::from_static("x-mbx-apikey"),
            HeaderValue::from_str(self.api_key.as_str())?,
        );

        if self.verbose {
            eprintln!("x-mbx-apikey: {}", self.api_key_redacted);
        }

        Ok(custom_headers)
    }

    async fn handler<T: DeserializeOwned>(&self, response: Response) -> Result<T> {
        match response.status() {
            StatusCode::OK => {
                let headers = response.headers().clone();
                let response_bytes = response.bytes().await?;

                if self.verbose {
                    println!("Response Headers: {:?}", headers);
                    let pretty =
                        serde_json::from_slice::<serde_json::Value>(&response_bytes).unwrap();
                    println!("Response: {}", pretty);
                }
                let json: T = serde_json::from_slice(&response_bytes)?;
                Ok(json)
            }
            StatusCode::INTERNAL_SERVER_ERROR => {
                bail!("Internal Server Error");
            }
            StatusCode::SERVICE_UNAVAILABLE => {
                bail!("Service Unavailable");
            }
            StatusCode::UNAUTHORIZED => {
                bail!("Unauthorized");
            }
            StatusCode::BAD_REQUEST => {
                let error: BinanceContentError = response.json().await?;

                Err(ErrorKind::BinanceError(error).into())
            }
            s => {
                bail!(format!("Received response: {:?}", s));
            }
        }
    }
}
