use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BookTickerEvent, ContinuousKlineEvent, DayTickerEvent,
    DepthOrderBookEvent, IndexKlineEvent, IndexPriceEvent, KlineEvent, LiquidationEvent,
    MarkPriceEvent, MiniTickerEvent, OrderBook, TradeEvent, UserDataStreamExpiredEvent,
};
use crate::futures::model;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_tungstenite::tungstenite::Message;
use futures_util::{SinkExt, StreamExt};

// ======================================================================
// WebsocketAPI – URL builder
// ======================================================================

enum FuturesWebsocketAPI {
    /// Build URL from the [`FuturesMarket`] enum (hardcoded base URLs).
    Default,
    /// Build a combined-stream URL from the [`FuturesMarket`] enum.
    MultiStream,
    /// Build URL from a config-provided **base** URL (no routing path).
    ///
    /// The routing path (`/public` / `/market` / `/private`) and the
    /// stream name are appended automatically.
    Custom(String),
}

// ── Market enum ──────────────────────────────────────────────────────

/// Supported futures market types.
#[derive(Debug, Clone, Copy)]
pub enum FuturesMarket {
    USDM,
    COINM,
    Vanilla,
    USDMTestnet,
    COINMTestnet,
    VanillaTestnet,
}

// ── Route helpers ────────────────────────────────────────────────────

/// Determine the routed endpoint path for a futures stream name.
///
/// See the [Binance WebSocket change notice] for the full mapping.
///
/// [Binance WebSocket change notice]: https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams
fn stream_route(subscription: &str) -> &'static str {
    // Listen-key-based user data streams have no `@`.
    if !subscription.contains('@') {
        return "/private";
    }

    // High-frequency order-book streams → Public route.
    if subscription.contains("depth") || subscription.contains("bookTicker") {
        return "/public";
    }

    // Everything else → Market route.
    "/market"
}

/// Return the base URL for a [`FuturesMarket`] (without trailing slash).
fn market_base_url(market: &FuturesMarket) -> &'static str {
    match market {
        FuturesMarket::USDM => "wss://fstream.binance.com",
        FuturesMarket::COINM => "wss://dstream.binance.com",
        FuturesMarket::Vanilla => "wss://vstream.binance.com",
        FuturesMarket::USDMTestnet => "wss://fstream.binancefuture.com",
        FuturesMarket::COINMTestnet => "wss://dstream.binancefuture.com",
        FuturesMarket::VanillaTestnet => "wss://vstream.binancefuture.com",
    }
}

// ── URL builder ──────────────────────────────────────────────────────

impl FuturesWebsocketAPI {
    /// Build a fully-qualified WebSocket URL.
    ///
    /// # Arguments
    ///
    /// * `market` – Market variant (used for `Default` / `MultiStream`).
    /// * `subscription` – The stream name(s), e.g. `"btcusdt@aggTrade"`.
    fn build_url(&self, market: &FuturesMarket, subscription: &str) -> String {
        match self {
            FuturesWebsocketAPI::Default => {
                let base = market_base_url(market);
                let route = stream_route(subscription);
                format!("{base}{route}/ws/{subscription}")
            }
            FuturesWebsocketAPI::MultiStream => {
                let base = market_base_url(market);
                // For multi-stream, pick the route from the first stream
                // (all streams in a single connection must share the same route).
                let route = stream_route(subscription);
                format!("{base}{route}/stream?streams={subscription}")
            }
            FuturesWebsocketAPI::Custom(base_url) => {
                let base = base_url.trim_end_matches('/');
                let route = stream_route(subscription);
                format!("{base}{route}/ws/{subscription}")
            }
        }
    }
}

// ======================================================================
// Event types
// ======================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FuturesWebsocketEvent {
    AccountUpdate(AccountUpdateEvent),
    OrderTrade(model::OrderTradeEvent),
    AggrTrades(AggrTradesEvent),
    Trade(TradeEvent),
    OrderBook(OrderBook),
    DayTicker(DayTickerEvent),
    MiniTicker(MiniTickerEvent),
    MiniTickerAll(Vec<MiniTickerEvent>),
    IndexPrice(IndexPriceEvent),
    MarkPrice(MarkPriceEvent),
    MarkPriceAll(Vec<MarkPriceEvent>),
    DayTickerAll(Vec<DayTickerEvent>),
    Kline(KlineEvent),
    ContinuousKline(ContinuousKlineEvent),
    IndexKline(IndexKlineEvent),
    Liquidation(LiquidationEvent),
    DepthOrderBook(DepthOrderBookEvent),
    BookTicker(BookTickerEvent),
    UserDataStreamExpiredEvent(UserDataStreamExpiredEvent),
}

// ======================================================================
// WebSockets – low-level connection & event loop
// ======================================================================

pub struct FuturesWebSockets<'a> {
    pub(crate) ws_client: Option<crate::ws_core::WsClient>,
    handler: Box<dyn FnMut(FuturesWebsocketEvent) -> Result<()> + 'a + Send>,
}

// ── Internal JSON dispatch ──────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum FuturesEvents {
    Vec(Vec<DayTickerEvent>),
    DayTickerEvent(DayTickerEvent),
    BookTickerEvent(BookTickerEvent),
    MiniTickerEvent(MiniTickerEvent),
    VecMiniTickerEvent(Vec<MiniTickerEvent>),
    AccountUpdateEvent(AccountUpdateEvent),
    OrderTradeEvent(model::OrderTradeEvent),
    AggrTradesEvent(AggrTradesEvent),
    IndexPriceEvent(IndexPriceEvent),
    MarkPriceEvent(MarkPriceEvent),
    VecMarkPriceEvent(Vec<MarkPriceEvent>),
    TradeEvent(TradeEvent),
    KlineEvent(KlineEvent),
    ContinuousKlineEvent(ContinuousKlineEvent),
    IndexKlineEvent(IndexKlineEvent),
    LiquidationEvent(LiquidationEvent),
    OrderBook(OrderBook),
    DepthOrderBookEvent(DepthOrderBookEvent),
    UserDataStreamExpiredEvent(UserDataStreamExpiredEvent),
}

// ── Constructor & connection helpers ─────────────────────────────────

impl<'a> FuturesWebSockets<'a> {
    pub fn new<Callback>(handler: Callback) -> FuturesWebSockets<'a>
    where
        Callback: FnMut(FuturesWebsocketEvent) -> Result<()> + 'a + Send,
    {
        FuturesWebSockets {
            ws_client: None,
            handler: Box::new(handler),
        }
    }

    /// Connect to a **single** stream, inferring the route from the
    /// stream name.
    pub async fn connect(&mut self, market: &FuturesMarket, subscription: &str) -> Result<()> {
        let url = FuturesWebsocketAPI::Default.build_url(market, subscription);
        self.connect_wss(&url).await
    }

    /// Connect to a single stream using a [`Config`] for the base URL
    /// instead of the hardcoded market URLs.
    pub async fn connect_with_config(
        &mut self, _market: &FuturesMarket, subscription: &str, config: &Config,
    ) -> Result<()> {
        let base = config.futures_ws_endpoint.clone();
        let url = FuturesWebsocketAPI::Custom(base).build_url(_market, subscription);
        self.connect_wss(&url).await
    }

    /// Connect to **multiple** streams on a single connection.
    ///
    /// All streams must belong to the **same route** (Public / Market /
    /// Private); the route is inferred from the first stream.
    pub async fn connect_multiple_streams(
        &mut self, market: &FuturesMarket, endpoints: &[String],
    ) -> Result<()> {
        let streams = endpoints.join("/");
        let url = FuturesWebsocketAPI::MultiStream.build_url(market, &streams);
        self.connect_wss(&url).await
    }

    async fn connect_wss(&mut self, wss: &str) -> Result<()> {
        self.ws_client = Some(crate::ws_core::WsClient::connect(wss).await?);
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut client) = self.ws_client.take() {
            client.disconnect().await?;
            Ok(())
        } else {
            Err(crate::errors::Error::Msg(
                "Not able to close the connection".to_string(),
            ))
        }
    }
}

// ── Message dispatching ──────────────────────────────────────────────

impl<'a> FuturesWebSockets<'a> {
    pub fn test_handle_msg(&mut self, msg: &str) -> Result<()> {
        self.handle_msg(msg)
    }

    pub fn handle_msg(&mut self, msg: &str) -> Result<()> {
        let value: serde_json::Value = serde_json::from_str(msg)?;
        self.handle_value(value)
    }

    fn handle_value(&mut self, value: serde_json::Value) -> Result<()> {
        // Unwrap `{"stream":"...","data":<rawPayload>}` wrapper.
        if let Some(data) = value.get("data") {
            return self.handle_value(data.clone());
        }

        if let Ok(events) = serde_json::from_value::<FuturesEvents>(value) {
            let action = match events {
                FuturesEvents::Vec(v) => FuturesWebsocketEvent::DayTickerAll(v),
                FuturesEvents::DayTickerEvent(v) => FuturesWebsocketEvent::DayTicker(v),
                FuturesEvents::BookTickerEvent(v) => FuturesWebsocketEvent::BookTicker(v),
                FuturesEvents::MiniTickerEvent(v) => FuturesWebsocketEvent::MiniTicker(v),
                FuturesEvents::VecMiniTickerEvent(v) => FuturesWebsocketEvent::MiniTickerAll(v),
                FuturesEvents::AccountUpdateEvent(v) => FuturesWebsocketEvent::AccountUpdate(v),
                FuturesEvents::OrderTradeEvent(v) => FuturesWebsocketEvent::OrderTrade(v),
                FuturesEvents::IndexPriceEvent(v) => FuturesWebsocketEvent::IndexPrice(v),
                FuturesEvents::MarkPriceEvent(v) => FuturesWebsocketEvent::MarkPrice(v),
                FuturesEvents::VecMarkPriceEvent(v) => FuturesWebsocketEvent::MarkPriceAll(v),
                FuturesEvents::TradeEvent(v) => FuturesWebsocketEvent::Trade(v),
                FuturesEvents::ContinuousKlineEvent(v) => FuturesWebsocketEvent::ContinuousKline(v),
                FuturesEvents::IndexKlineEvent(v) => FuturesWebsocketEvent::IndexKline(v),
                FuturesEvents::LiquidationEvent(v) => FuturesWebsocketEvent::Liquidation(v),
                FuturesEvents::KlineEvent(v) => FuturesWebsocketEvent::Kline(v),
                FuturesEvents::OrderBook(v) => FuturesWebsocketEvent::OrderBook(v),
                FuturesEvents::DepthOrderBookEvent(v) => FuturesWebsocketEvent::DepthOrderBook(v),
                FuturesEvents::AggrTradesEvent(v) => FuturesWebsocketEvent::AggrTrades(v),
                FuturesEvents::UserDataStreamExpiredEvent(v) => {
                    FuturesWebsocketEvent::UserDataStreamExpiredEvent(v)
                }
            };
            (self.handler)(action)?;
        }
        Ok(())
    }
}

// ── Event loop ───────────────────────────────────────────────────────

impl<'a> FuturesWebSockets<'a> {
    pub async fn event_loop(&mut self, running: &AtomicBool) -> Result<()> {
        while running.load(Ordering::Acquire) {
            if let Some(ref mut socket) = self.ws_client.as_mut().and_then(|c| c.socket.as_mut()) {
                match socket.next().await {
                    Some(Ok(message)) => match message {
                        Message::Text(msg) => {
                            if let Err(e) = self.handle_msg(&msg) {
                                return Err(crate::errors::Error::Msg(format!(
                                    "Error on handling stream message: {}",
                                    e
                                )));
                            }
                        }
                        Message::Ping(data) => {
                            let _ = socket.send(Message::Pong(data)).await;
                        }
                        Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => (),
                        Message::Close(e) => {
                            return Err(crate::errors::Error::Msg(format!("Disconnected {:?}", e)))
                        }
                    },
                    Some(Err(e)) => {
                        return Err(crate::errors::Error::Msg(format!("WebSocket error: {}", e)))
                    }
                    None => {
                        return Err(crate::errors::Error::Msg(
                            "WebSocket stream ended".to_string(),
                        ))
                    }
                }
            } else {
                return Err(crate::errors::Error::Msg(
                    "WebSocket is not connected".to_string(),
                ));
            }
        }
        Ok(())
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Route detection ──────────────────────────────────────────────

    #[test]
    fn depth_stream_goes_to_public() {
        assert_eq!(stream_route("btcusdt@depth"), "/public");
    }

    #[test]
    fn depth_levels_stream_goes_to_public() {
        assert_eq!(stream_route("btcusdt@depth20@100ms"), "/public");
    }

    #[test]
    fn book_ticker_goes_to_public() {
        assert_eq!(stream_route("btcusdt@bookTicker"), "/public");
    }

    #[test]
    fn agg_trade_goes_to_market() {
        assert_eq!(stream_route("btcusdt@aggTrade"), "/market");
    }

    #[test]
    fn mark_price_goes_to_market() {
        assert_eq!(stream_route("btcusdt@markPrice"), "/market");
    }

    #[test]
    fn kline_goes_to_market() {
        assert_eq!(stream_route("btcusdt@kline_1m"), "/market");
    }

    #[test]
    fn force_order_goes_to_market() {
        assert_eq!(stream_route("btcusdt@forceOrder"), "/market");
    }

    #[test]
    fn listen_key_goes_to_private() {
        assert_eq!(
            stream_route("AbCdEf1234567890abcdefghijklmnopqrstuvwxyz1234567890"),
            "/private"
        );
    }

    // ── URL building ─────────────────────────────────────────────────

    #[test]
    fn default_single_stream() {
        let url = FuturesWebsocketAPI::Default.build_url(&FuturesMarket::USDM, "btcusdt@aggTrade");
        assert_eq!(url, "wss://fstream.binance.com/market/ws/btcusdt@aggTrade");
    }

    #[test]
    fn default_public_stream() {
        let url = FuturesWebsocketAPI::Default.build_url(&FuturesMarket::USDM, "btcusdt@depth");
        assert_eq!(url, "wss://fstream.binance.com/public/ws/btcusdt@depth");
    }

    #[test]
    fn multi_stream() {
        let url = FuturesWebsocketAPI::MultiStream
            .build_url(&FuturesMarket::USDM, "btcusdt@aggTrade/ethusdt@markPrice");
        assert_eq!(
            url,
            "wss://fstream.binance.com/market/stream?streams=btcusdt@aggTrade/ethusdt@markPrice"
        );
    }

    #[test]
    fn custom_base_url() {
        let url = FuturesWebsocketAPI::Custom("wss://fstream.binance.com".into())
            .build_url(&FuturesMarket::USDM, "btcusdt@markPrice");
        assert_eq!(url, "wss://fstream.binance.com/market/ws/btcusdt@markPrice");
    }

    #[test]
    fn custom_private_stream() {
        let url = FuturesWebsocketAPI::Custom("wss://fstream.binance.com".into())
            .build_url(&FuturesMarket::USDM, "my-listen-key");
        assert_eq!(url, "wss://fstream.binance.com/private/ws/my-listen-key");
    }

    #[test]
    fn custom_base_with_trailing_slash() {
        let url = FuturesWebsocketAPI::Custom("wss://example.com/".into())
            .build_url(&FuturesMarket::USDM, "btcusdt@trade");
        assert_eq!(url, "wss://example.com/market/ws/btcusdt@trade");
    }
}
