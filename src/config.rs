//! Configuration for Binance API endpoints.
//!
//! Provides defaults for mainnet and testnet, plus a builder-style
//! [`Config`] struct that is used throughout the library.

/// Application-level configuration.
///
/// Create with [`Config::default()`] (mainnet) or [`Config::testnet()`],
/// then override individual fields via the builder methods.
#[derive(Clone, Debug)]
pub struct Config {
    // ── Spot ──────────────────────────────────────────────────────────
    pub rest_api_endpoint: String,
    pub ws_endpoint: String,

    // ── USDⓈ-M Futures ───────────────────────────────────────────────
    pub futures_rest_api_endpoint: String,
    /// Base URL for futures WebSocket (without a routing path).
    ///
    /// The actual connection URL is built by appending a routing path
    /// (`/public`, `/market` or `/private`) plus the stream mode
    /// (`/ws/<stream>` or `/stream?streams=...`).
    ///
    /// See [`Config::futures_ws_public_url`],
    /// [`Config::futures_ws_market_url`] and
    /// [`Config::futures_ws_private_url`].
    pub futures_ws_endpoint: String,

    pub recv_window: u64,
}

// ── Route path constants ─────────────────────────────────────────────

/// Routed path for **Public** (high-frequency) market data.
///
/// Streams served here: `depth`, `depth@100ms`, `bookTicker` and
/// `diffDepth` variants.
pub const FUTURES_WS_ROUTE_PUBLIC: &str = "/public";

/// Routed path for **Market** (regular-frequency) market data.
///
/// Streams served here: `aggTrade`, `trade`, `kline`, `markPrice`,
/// `indexPrice`, `ticker`, `miniTicker`, `forceOrder`,
/// `continuousKline`, `indexPriceKline` and their `@arr` variants.
pub const FUTURES_WS_ROUTE_MARKET: &str = "/market";

/// Routed path for **Private** user data.
///
/// Streams served here: listen-key based user data streams.
pub const FUTURES_WS_ROUTE_PRIVATE: &str = "/private";

// ── Default endpoint constants ───────────────────────────────────────

// Spot
pub const SPOT_MAINNET: &str = "https://api.binance.com";
pub const SPOT_TESTNET: &str = "https://testnet.binance.vision";

pub const SPOT_WS_MAINNET: &str = "wss://stream.binance.com/ws";
pub const SPOT_WS_TESTNET: &str = "wss://testnet.binance.vision/ws";

// USDⓈ-M Futures
pub const FUTURES_MAINNET: &str = "https://fapi.binance.com";
pub const FUTURES_TESTNET: &str = "https://testnet.binancefuture.com";

/// Base WebSocket URL for USDⓈ-M Futures mainnet (no routing path).
pub const FUTURES_WS_MAINNET: &str = "wss://fstream.binance.com";

/// Base WebSocket URL for USDⓈ-M Futures testnet (no routing path).
pub const FUTURES_WS_TESTNET: &str = "wss://fstream.binancefuture.com";

// ── Default impl ─────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            rest_api_endpoint: SPOT_MAINNET.into(),
            ws_endpoint: SPOT_WS_MAINNET.into(),

            futures_rest_api_endpoint: FUTURES_MAINNET.into(),
            futures_ws_endpoint: FUTURES_WS_MAINNET.into(),

            recv_window: 5000,
        }
    }
}

// ── Builder & helper methods ─────────────────────────────────────────

impl Config {
    // ── Presets ───────────────────────────────────────────────────────

    /// Return a [`Config`] pre-populated with all testnet endpoints.
    pub fn testnet() -> Self {
        Self::default()
            .set_rest_api_endpoint(SPOT_TESTNET)
            .set_ws_endpoint(SPOT_WS_TESTNET)
            .set_futures_rest_api_endpoint(FUTURES_TESTNET)
            .set_futures_ws_endpoint(FUTURES_WS_TESTNET)
    }

    // ── Spot REST / WS ───────────────────────────────────────────────

    pub fn set_rest_api_endpoint<T: Into<String>>(mut self, rest_api_endpoint: T) -> Self {
        self.rest_api_endpoint = rest_api_endpoint.into();
        self
    }

    pub fn set_ws_endpoint<T: Into<String>>(mut self, ws_endpoint: T) -> Self {
        self.ws_endpoint = ws_endpoint.into();
        self
    }

    // ── Futures REST / WS ────────────────────────────────────────────

    pub fn set_futures_rest_api_endpoint<T: Into<String>>(
        mut self, futures_rest_api_endpoint: T,
    ) -> Self {
        self.futures_rest_api_endpoint = futures_rest_api_endpoint.into();
        self
    }

    /// Set the base URL for futures WebSocket connections.
    ///
    /// This should be the **base** URL **without** a routing path or
    /// stream mode suffix (e.g. `"wss://fstream.binance.com"`).
    pub fn set_futures_ws_endpoint<T: Into<String>>(mut self, futures_ws_endpoint: T) -> Self {
        self.futures_ws_endpoint = futures_ws_endpoint.into();
        self
    }

    // ── Recv window ─────────────────────────────────────────────────

    pub fn set_recv_window(mut self, recv_window: u64) -> Self {
        self.recv_window = recv_window;
        self
    }

    // ── Futures routed URL helpers ───────────────────────────────────

    /// Build the full WebSocket URL for the **public** route.
    ///
    /// Mode `"ws"` → `{base}/public/ws/{stream}`
    /// Mode `"stream"` → `{base}/public/stream?streams={streams}`
    pub fn futures_ws_public_url(&self, stream_or_streams: &str, mode: &str) -> String {
        self.futures_routed_url(FUTURES_WS_ROUTE_PUBLIC, stream_or_streams, mode)
    }

    /// Build the full WebSocket URL for the **market** route.
    ///
    /// Mode `"ws"` → `{base}/market/ws/{stream}`
    /// Mode `"stream"` → `{base}/market/stream?streams={streams}`
    pub fn futures_ws_market_url(&self, stream_or_streams: &str, mode: &str) -> String {
        self.futures_routed_url(FUTURES_WS_ROUTE_MARKET, stream_or_streams, mode)
    }

    /// Build the full WebSocket URL for the **private** route.
    ///
    /// Mode `"ws"` → `{base}/private/ws/{listenKey}`
    /// Mode `"stream"` → `{base}/private/stream?streams={streams}`
    pub fn futures_ws_private_url(&self, stream_or_streams: &str, mode: &str) -> String {
        self.futures_routed_url(FUTURES_WS_ROUTE_PRIVATE, stream_or_streams, mode)
    }

    // ── Internal ─────────────────────────────────────────────────────

    /// Generic routed URL builder.
    fn futures_routed_url(&self, route: &str, stream_or_streams: &str, mode: &str) -> String {
        let base = self.futures_ws_endpoint.trim_end_matches('/');
        match mode {
            "ws" => format!("{base}{route}/ws/{stream_or_streams}"),
            "stream" => {
                format!("{base}{route}/stream?streams={stream_or_streams}")
            }
            other => {
                // Fallback – treat as a custom/raw path segment
                format!("{base}{route}/{other}/{stream_or_streams}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_mainnet() {
        let cfg = Config::default();
        assert_eq!(cfg.rest_api_endpoint, SPOT_MAINNET);
        assert_eq!(cfg.futures_ws_endpoint, FUTURES_WS_MAINNET);
    }

    #[test]
    fn testnet_uses_testnet_urls() {
        let cfg = Config::testnet();
        assert_eq!(cfg.rest_api_endpoint, SPOT_TESTNET);
        assert_eq!(cfg.futures_ws_endpoint, FUTURES_WS_TESTNET);
    }

    #[test]
    fn public_ws_url() {
        let cfg = Config::default();
        let url = cfg.futures_ws_public_url("btcusdt@depth", "ws");
        assert_eq!(url, "wss://fstream.binance.com/public/ws/btcusdt@depth");
    }

    #[test]
    fn market_ws_url() {
        let cfg = Config::default();
        let url = cfg.futures_ws_market_url("btcusdt@markPrice", "ws");
        assert_eq!(url, "wss://fstream.binance.com/market/ws/btcusdt@markPrice");
    }

    #[test]
    fn private_ws_url() {
        let cfg = Config::default();
        let url = cfg.futures_ws_private_url("my-listen-key", "ws");
        assert_eq!(url, "wss://fstream.binance.com/private/ws/my-listen-key");
    }

    #[test]
    fn market_stream_url() {
        let cfg = Config::default();
        let url = cfg.futures_ws_market_url("btcusdt@aggTrade/ethusdt@markPrice", "stream");
        assert_eq!(
            url,
            "wss://fstream.binance.com/market/stream?streams=btcusdt@aggTrade/ethusdt@markPrice"
        );
    }

    #[test]
    fn testnet_public_ws_url() {
        let cfg = Config::testnet();
        let url = cfg.futures_ws_public_url("btcusdt@depth", "ws");
        assert_eq!(
            url,
            "wss://fstream.binancefuture.com/public/ws/btcusdt@depth"
        );
    }

    #[test]
    fn builder_pattern_is_consumable() {
        let cfg = Config::default()
            .set_futures_ws_endpoint("wss://custom.example.com")
            .set_recv_window(10_000);
        assert_eq!(cfg.futures_ws_endpoint, "wss://custom.example.com");
        assert_eq!(cfg.recv_window, 10_000);
    }
}
