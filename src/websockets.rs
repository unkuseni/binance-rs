use crate::errors::Result;
use crate::model::{
    AggrTradesEvent, BookTickerEvent, DayTickerEvent, WindowTickerEvent, DepthOrderBookEvent,
    KlineEvent, OrderBook, TradeEvent,
};
use crate::client::Client;
use crate::websockets_old::{WebSockets, WebsocketEvent};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::atomic::AtomicU64;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use std::time::Duration;

/// Atomic counter for generating unique IDs for subscribe/unsubscribe requests.
static WS_REQ_ID: AtomicU64 = AtomicU64::new(1);

// ============================================
// New Stream API - Simplified high-level interface for WebSocket subscriptions
// ============================================

/// Market type for WebSocket connections
#[derive(Debug, Clone, Copy)]
pub enum Market {
    Spot,
    SpotTestnet,
}

impl Market {
    pub fn base_url(&self) -> &'static str {
        match self {
            Market::Spot => "wss://stream.binance.com",
            Market::SpotTestnet => "wss://testnet.binance.vision",
        }
    }
}

/// Stream configuration
#[derive(Clone)]
pub struct StreamConfig {
    pub market: Market,
    pub client: Option<Client>,
    pub recv_window: Option<u64>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            market: Market::Spot,
            client: None,
            recv_window: None,
        }
    }
}

/// High-level WebSocket stream interface (similar to Bybit's Stream)
#[derive(Clone)]
pub struct Stream {
    config: StreamConfig,
}

impl Default for Stream {
    fn default() -> Self {
        Self::new()
    }
}

/// Commands for dynamic WebSocket subscription management.
#[derive(Debug, Clone)]
pub enum StreamCommand {
    /// Subscribe to a new stream (e.g., "btcusdt@trade")
    Subscribe(String),
    /// Unsubscribe from a stream
    Unsubscribe(String),
}

impl Stream {
    /// Create a new Stream with default configuration (public streams only)
    pub fn new() -> Self {
        Self {
            config: StreamConfig::default(),
        }
    }

    /// Create a new Stream with a client for private streams
    pub fn with_client(client: Client) -> Self {
        Self {
            config: StreamConfig {
                client: Some(client),
                ..Default::default()
            },
        }
    }

    /// Create a new Stream with custom configuration
    pub fn with_config(config: StreamConfig) -> Self {
        Self { config }
    }

    /// Subscribe to a single stream with a callback handler
    ///
    /// # Arguments
    ///
    /// * `subscription` - Stream name (e.g., "btcusdt@trade")
    /// * `handler` - Callback function to handle events
    ///
    /// # Example
    ///
    /// ```no_run
    /// use binance::websockets::*;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let stream = Stream::new();
    ///     let keep_running = Arc::new(AtomicBool::new(true));
    ///
    ///     tokio::spawn(async move {
    ///         stream.ws_subscribe("btcusdt@trade", |event| {
    ///             match event {
    ///                 WebsocketEvent::Trade(trade) => {
    ///                     println!("Trade: {} @ {}", trade.symbol, trade.price);
    ///                 }
    ///                 _ => {}
    ///             }
    ///             Ok(())
    ///         }).await.unwrap();
    ///     });
    /// }
    /// ```
    pub async fn ws_subscribe<F>(&self, subscription: &str, handler: F) -> Result<()>
    where
        F: FnMut(WebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = WebSockets::new(handler);
        ws.connect_with_market(self.config.market, subscription)
            .await?;
        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }

    /// Subscribe to multiple streams with a callback handler
    ///
    /// # Arguments
    ///
    /// * `streams` - Vector of stream names (e.g., ["btcusdt@trade", "ethusdt@ticker"])
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe_multiple<F>(&self, streams: &[String], handler: F) -> Result<()>
    where
        F: FnMut(WebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = WebSockets::new(handler);
        ws.connect_multiple_streams(streams).await?;
        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }

    /// Subscribe to a single stream with a channel-based output.
    ///
    /// A generic helper that eliminates ~15 lines of boilerplate per channel method.
    /// Use this instead of manually calling `ws_subscribe` with a pattern-match closure.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The data type extracted from the event
    /// * `M` - A mapper closure: `WebsocketEvent -> Option<T>`
    async fn subscribe_channel<T, M>(
        &self, subscription: &str, sender: mpsc::UnboundedSender<T>, mapper: M,
    ) -> Result<()>
    where
        T: Send + 'static,
        M: Fn(WebsocketEvent) -> Option<T> + Send + 'static,
    {
        self.ws_subscribe(subscription, move |event| {
            if let Some(data) = mapper(event) {
                sender
                    .send(data)
                    .map_err(|e| crate::errors::Error::Msg(format!("Channel send error: {}", e)))?;
            }
            Ok(())
        })
        .await
    }

    /// Subscribe to order book depth updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `depth` - Order book depth (5, 10, 20, 50, 100, 500, 1000, 5000)
    /// * `speed` - Update speed ("100ms" or "1000ms")
    /// * `sender` - Channel sender for order book updates
    pub async fn ws_orderbook(
        &self, symbol: &str, depth: u32, speed: &str, sender: mpsc::UnboundedSender<OrderBook>,
    ) -> Result<()> {
        let subscription = format!("{}@depth{}@{}", symbol.to_lowercase(), depth, speed);
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::OrderBook(book) = event {
                Some(book)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to trade updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for trade updates
    pub async fn ws_trades(
        &self, symbol: &str, sender: mpsc::UnboundedSender<TradeEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@trade", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::Trade(trade) = event {
                Some(trade)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to kline/candlestick updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `interval` - Kline interval (e.g., "1m", "5m", "1h", "1d")
    /// * `sender` - Channel sender for kline updates
    pub async fn ws_klines(
        &self, symbol: &str, interval: &str, sender: mpsc::UnboundedSender<KlineEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@kline_{}", symbol.to_lowercase(), interval);
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::Kline(kline) = event {
                Some(kline)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for ticker updates
    pub async fn ws_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<DayTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@ticker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::DayTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to mini ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for mini ticker updates
    pub async fn ws_mini_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<WindowTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@miniTicker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::WindowTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to book ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for book ticker updates
    pub async fn ws_book_ticker(
        &self, symbol: &str, sender: mpsc::UnboundedSender<BookTickerEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@bookTicker", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::BookTicker(ticker) = event {
                Some(ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all ticker updates (Vec)
    pub async fn ws_all_tickers(
        &self, sender: mpsc::UnboundedSender<Vec<DayTickerEvent>>,
    ) -> Result<()> {
        let subscription = "!ticker@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let WebsocketEvent::DayTickerAll(tickers) = event {
                Some(tickers)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all mini ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all mini ticker updates (Vec)
    pub async fn ws_all_mini_tickers(
        &self, sender: mpsc::UnboundedSender<Vec<WindowTickerEvent>>,
    ) -> Result<()> {
        let subscription = "!miniTicker@arr";
        self.subscribe_channel(subscription, sender, |event| {
            if let WebsocketEvent::WindowTickerAll(mini_tickers) = event {
                Some(mini_tickers)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to all book ticker updates with channel output
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for all book ticker updates
    pub async fn ws_all_book_tickers(
        &self, sender: mpsc::UnboundedSender<BookTickerEvent>,
    ) -> Result<()> {
        let subscription = "!bookTicker";
        self.subscribe_channel(subscription, sender, |event| {
            if let WebsocketEvent::BookTicker(book_ticker) = event {
                Some(book_ticker)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to aggregate trades updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for aggregate trade updates
    pub async fn ws_agg_trades(
        &self, symbol: &str, sender: mpsc::UnboundedSender<AggrTradesEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@aggTrade", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::AggrTrades(agg_trade) = event {
                Some(agg_trade)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to depth order book updates with channel output
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `sender` - Channel sender for depth order book updates
    pub async fn ws_depth_orderbook(
        &self, symbol: &str, sender: mpsc::UnboundedSender<DepthOrderBookEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@depth", symbol.to_lowercase());
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                Some(depth_orderbook)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to depth order book updates with specific levels and speed
    ///
    /// # Arguments
    ///
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `levels` - Order book levels (5, 10, 20, 50, 100, 500, 1000, 5000)
    /// * `speed` - Update speed ("100ms" or "1000ms")
    /// * `sender` - Channel sender for depth order book updates
    pub async fn ws_depth_orderbook_levels(
        &self, symbol: &str, levels: u32, speed: &str,
        sender: mpsc::UnboundedSender<DepthOrderBookEvent>,
    ) -> Result<()> {
        let subscription = format!("{}@depth{}@{}", symbol.to_lowercase(), levels, speed);
        self.subscribe_channel(&subscription, sender, |event| {
            if let WebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                Some(depth_orderbook)
            } else {
                None
            }
        })
        .await
    }

    /// Subscribe to user data stream (private)
    ///
    /// # Arguments
    ///
    /// * `listen_key` - User data stream listen key
    /// * `sender` - Channel sender for user data events
    ///
    /// # Note
    ///
    /// Requires a client with API key and secret
    pub async fn ws_user_data(
        &self, listen_key: &str, sender: mpsc::UnboundedSender<WebsocketEvent>,
    ) -> Result<()> {
        let subscription = listen_key;
        self.subscribe_channel(subscription, sender, Some).await
    }

    /// Subscribe to user data stream with automatic listen key management
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender for user data events
    ///
    /// # Note
    ///
    /// Requires a client with API key and secret
    pub async fn ws_user_data_auto(
        &self, sender: mpsc::UnboundedSender<WebsocketEvent>,
    ) -> Result<()> {
        let client = self.config.client.as_ref().ok_or_else(|| {
            crate::errors::Error::Msg("Client required for user data stream".to_string())
        })?;

        // Start user data stream
        use crate::userstream::UserStream;
        let user_stream = UserStream {
            client: client.clone(),
            recv_window: self.config.recv_window.unwrap_or(5000),
        };

        let listen_key_data = user_stream.start().await?;
        let listen_key = listen_key_data.listen_key;

        // Spawn keep-alive task
        let keep_alive_client = client.clone();
        let recv_window = self.config.recv_window.unwrap_or(5000);
        let listen_key_clone = listen_key.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30 * 60)); // 30 minutes
            let user_stream = UserStream {
                client: keep_alive_client,
                recv_window,
            };

            loop {
                interval.tick().await;
                if let Err(e) = user_stream.keep_alive(&listen_key_clone).await {
                    eprintln!("Failed to keep alive user stream: {}", e);
                }
            }
        });

        self.ws_user_data(&listen_key, sender).await
    }

    /// Subscribe to streams with dynamic runtime subscription management.
    ///
    /// Unlike `ws_subscribe` which subscribes once, this allows adding/removing
    /// subscriptions at runtime via the command channel **without reconnecting**.
    /// Uses Binance's native `SUBSCRIBE`/`UNSUBSCRIBE` protocol messages over
    /// the existing WebSocket connection.
    ///
    /// # Arguments
    ///
    /// * `initial_streams` - Initial set of stream names to subscribe to
    /// * `cmd_rx` - Channel receiver for dynamic subscribe/unsubscribe commands
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe_with_commands<F>(
        &self, initial_streams: &[String], mut cmd_rx: mpsc::UnboundedReceiver<StreamCommand>,
        handler: F,
    ) -> Result<()>
    where
        F: FnMut(WebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut active: HashSet<String> = initial_streams.iter().cloned().collect();

        // Wait until we have at least one stream to connect
        if active.is_empty() {
            loop {
                match cmd_rx.recv().await {
                    Some(StreamCommand::Subscribe(s)) => {
                        active.insert(s);
                        break;
                    }
                    Some(StreamCommand::Unsubscribe(_)) => continue,
                    None => return Ok(()),
                }
            }
        }

        // Connect once with the initial stream set
        let streams: Vec<String> = active.iter().cloned().collect();
        let mut ws = WebSockets::new(handler);
        ws.connect_multiple_streams(&streams).await?;

        // Event loop — commands are handled by sending protocol messages
        // over the existing socket instead of reconnecting.
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();

        while r.load(Ordering::Acquire) {
            // Check for commands before polling socket
            match cmd_rx.try_recv() {
                Ok(StreamCommand::Subscribe(s)) => {
                    if !active.contains(&s) {
                        let id = WS_REQ_ID.fetch_add(1, Ordering::Relaxed);
                        let msg = serde_json::json!({
                            "method": "SUBSCRIBE",
                            "params": [s],
                            "id": id,
                        });
                        let client = ws.ws_client.as_mut().ok_or_else(|| {
                            crate::errors::Error::Msg("WebSocket is not connected".to_string())
                        })?;
                        let socket = client.socket.as_mut().ok_or_else(|| {
                            crate::errors::Error::Msg("WebSocket is not connected".to_string())
                        })?;
                        socket
                            .send(Message::Text(msg.to_string().into()))
                            .await
                            .map_err(|e| {
                                crate::errors::Error::Msg(format!(
                                    "Failed to send SUBSCRIBE: {}",
                                    e
                                ))
                            })?;
                        active.insert(s);
                    }
                }
                Ok(StreamCommand::Unsubscribe(s)) => {
                    if active.contains(&s) {
                        let id = WS_REQ_ID.fetch_add(1, Ordering::Relaxed);
                        let msg = serde_json::json!({
                            "method": "UNSUBSCRIBE",
                            "params": [s],
                            "id": id,
                        });
                        let client = ws.ws_client.as_mut().ok_or_else(|| {
                            crate::errors::Error::Msg("WebSocket is not connected".to_string())
                        })?;
                        let socket = client.socket.as_mut().ok_or_else(|| {
                            crate::errors::Error::Msg("WebSocket is not connected".to_string())
                        })?;
                        socket
                            .send(Message::Text(msg.to_string().into()))
                            .await
                            .map_err(|e| {
                                crate::errors::Error::Msg(format!(
                                    "Failed to send UNSUBSCRIBE: {}",
                                    e
                                ))
                            })?;
                        active.remove(&s);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Channel closed, stop the event loop
                    return Ok(());
                }
            }

            // Poll socket with timeout so commands are checked periodically
            let client = ws.ws_client.as_mut().ok_or_else(|| {
                crate::errors::Error::Msg("WebSocket is not connected".to_string())
            })?;
            let socket = client.socket.as_mut().ok_or_else(|| {
                crate::errors::Error::Msg("WebSocket is not connected".to_string())
            })?;

            match tokio::time::timeout(Duration::from_millis(100), socket.next()).await {
                Ok(Some(Ok(Message::Text(msg)))) => {
                    // Check if this is a subscription response (has "id" field)
                    // rather than a data event. Binance sends:
                    //   {"result": null, "id": 1}            — success
                    //   {"code": ..., "msg": "...", "id": 1} — error
                    let value: serde_json::Value = serde_json::from_str(&msg).map_err(|e| {
                        crate::errors::Error::Msg(format!("JSON parse error: {}", e))
                    })?;
                    if value.get("id").and_then(|v| v.as_u64()).is_some() {
                        // Subscription response — check for errors
                        if let Some(code) = value.get("code").and_then(|v| v.as_i64()) {
                            let err_msg = value
                                .get("msg")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            return Err(crate::errors::Error::Msg(format!(
                                "Subscription error ({}): {}",
                                code, err_msg
                            )));
                        }
                        // Success response (result: null), continue
                    } else {
                        // Regular data event — pass to handler
                        ws.handle_msg(&msg)?;
                    }
                }
                Ok(Some(Ok(Message::Ping(data)))) => {
                    socket
                        .send(Message::Pong(data))
                        .await
                        .map_err(|e| crate::errors::Error::Msg(format!("Pong error: {}", e)))?;
                }
                Ok(Some(Ok(Message::Close(e)))) => {
                    return Err(crate::errors::Error::Msg(format!("Disconnected {:?}", e)));
                }
                Ok(Some(Err(e))) => {
                    return Err(crate::errors::Error::Msg(format!("WebSocket error: {}", e)));
                }
                Ok(None) => {
                    return Err(crate::errors::Error::Msg(
                        "WebSocket stream ended".to_string(),
                    ));
                }
                Err(_) => {} // Timeout, loop back and check commands
                _ => {}
            }
        }
        Ok(())
    }
}
