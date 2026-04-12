use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BalanceUpdateEvent, BookTickerEvent, DayTickerEvent,
    WindowTickerEvent, DepthOrderBookEvent, KlineEvent, OrderBook, OrderTradeEvent, TradeEvent,
};
use crate::client::Client;
use url::Url;
use serde::{Deserialize, Serialize};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;

const PING_INTERVAL_SECONDS: u64 = 30;

// ============================================
// New Stream API (Bybit-style)
// ============================================

/// Market type for WebSocket connections
#[derive(Debug, Clone, Copy)]
pub enum Market {
    Spot,
    SpotTestnet,
}

impl Market {
    fn base_url(&self) -> &'static str {
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::OrderBook(book) = event {
                sender.send(book).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::Trade(trade) = event {
                sender.send(trade).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::Kline(kline) = event {
                sender.send(kline).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::DayTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::WindowTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::BookTicker(ticker) = event {
                sender.send(ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::DayTickerAll(tickers) = event {
                sender.send(tickers).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::WindowTickerAll(mini_tickers) = event {
                sender.send(mini_tickers).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::BookTicker(book_ticker) = event {
                sender.send(book_ticker).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::AggrTrades(agg_trade) = event {
                sender.send(agg_trade).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                sender.send(depth_orderbook).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(&subscription, move |event| {
            if let WebsocketEvent::DepthOrderBook(depth_orderbook) = event {
                sender.send(depth_orderbook).map_err(|e| {
                    crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                        "Channel send error: {}",
                        e
                    )))
                })?;
            }
            Ok(())
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
        self.ws_subscribe(subscription, move |event| {
            sender.send(event).map_err(|e| {
                crate::errors::Error::from(crate::errors::ErrorKind::Msg(format!(
                    "Channel send error: {}",
                    e
                )))
            })?;
            Ok(())
        })
        .await
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
            crate::errors::ErrorKind::Msg("Client required for user data stream".to_string())
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

    /// Connect to WebSocket with dynamic command control
    ///
    /// # Arguments
    ///
    /// * `subscription` - Stream name
    /// * `cmd_receiver` - Channel receiver for commands
    /// * `handler` - Callback function to handle events
    pub async fn ws_subscribe_with_commands<'a, F>(
        &self, subscription: &str, cmd_receiver: mpsc::UnboundedReceiver<WebsocketCommand>,
        handler: F,
    ) -> Result<()>
    where
        F: FnMut(WebsocketEvent) -> Result<()> + 'static + Send,
    {
        let mut ws = WebSockets::new(handler);
        ws.connect(&self.build_url(subscription)).await?;

        // Transfer command receiver to WebSockets
        if let Some(ws_cmd_tx) = ws.get_command_sender() {
            tokio::spawn(async move {
                let mut cmd_rx = cmd_receiver;
                while let Some(cmd) = cmd_rx.recv().await {
                    let _ = ws_cmd_tx.send(cmd);
                }
            });
        }

        let running = Arc::new(AtomicBool::new(true));
        ws.event_loop(&running).await
    }

    fn build_url(&self, subscription: &str) -> String {
        let base = self.config.market.base_url();
        if subscription.contains('@') && !subscription.contains('?') {
            format!("{}/ws/{}", base, subscription)
        } else {
            // Assume it's already a full URL or multi-stream format
            if subscription.starts_with("wss://") {
                subscription.to_string()
            } else {
                format!("{}/stream?streams={}", base, subscription)
            }
        }
    }
}

// ============================================
// Legacy WebSockets API (maintained for compatibility)
// ============================================

enum WebsocketAPI {
    Default,
    MultiStream,
    Custom(String),
}

impl WebsocketAPI {
    fn params(self, subscription: &str) -> String {
        match self {
            WebsocketAPI::Default => format!("wss://stream.binance.com/ws/{}", subscription),
            WebsocketAPI::MultiStream => {
                format!("wss://stream.binance.com/stream?streams={}", subscription)
            }
            WebsocketAPI::Custom(url) => format!("{}/{}", url, subscription),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum WebsocketEvent {
    AccountUpdate(AccountUpdateEvent),
    BalanceUpdate(BalanceUpdateEvent),
    OrderTrade(OrderTradeEvent),
    AggrTrades(AggrTradesEvent),
    Trade(TradeEvent),
    OrderBook(OrderBook),
    DayTicker(DayTickerEvent),
    DayTickerAll(Vec<DayTickerEvent>),
    WindowTicker(WindowTickerEvent),
    WindowTickerAll(Vec<WindowTickerEvent>),
    Kline(KlineEvent),
    DepthOrderBook(DepthOrderBookEvent),
    BookTicker(BookTickerEvent),
}

pub struct WebSockets<'a> {
    pub socket: Option<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    handler: Box<dyn FnMut(WebsocketEvent) -> Result<()> + Send + 'a>,
    command_tx: Option<mpsc::UnboundedSender<WebsocketCommand>>,
    command_rx: Option<mpsc::UnboundedReceiver<WebsocketCommand>>,
}

#[derive(Debug)]
pub enum WebsocketCommand {
    Subscribe(String),
    Unsubscribe(String),
    Disconnect,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum Events {
    DayTickerEventAll(Vec<DayTickerEvent>),
    WindowTickerEventAll(Vec<WindowTickerEvent>),
    BalanceUpdateEvent(BalanceUpdateEvent),
    DayTickerEvent(DayTickerEvent),
    WindowTickerEvent(WindowTickerEvent),
    BookTickerEvent(BookTickerEvent),
    AccountUpdateEvent(AccountUpdateEvent),
    OrderTradeEvent(OrderTradeEvent),
    AggrTradesEvent(AggrTradesEvent),
    TradeEvent(TradeEvent),
    KlineEvent(KlineEvent),
    OrderBook(OrderBook),
    DepthOrderBookEvent(DepthOrderBookEvent),
}

impl<'a> WebSockets<'a> {
    pub fn new<Callback>(handler: Callback) -> WebSockets<'a>
    where
        Callback: FnMut(WebsocketEvent) -> Result<()> + 'a + Send,
    {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        WebSockets {
            socket: None,
            handler: Box::new(handler),
            command_tx: Some(command_tx),
            command_rx: Some(command_rx),
        }
    }

    pub async fn connect(&mut self, subscription: &str) -> Result<()> {
        self.connect_wss(&WebsocketAPI::Default.params(subscription))
            .await
    }

    pub async fn connect_with_config(&mut self, subscription: &str, config: &Config) -> Result<()> {
        self.connect_wss(&WebsocketAPI::Custom(config.ws_endpoint.clone()).params(subscription))
            .await
    }

    pub async fn connect_multiple_streams(&mut self, endpoints: &[String]) -> Result<()> {
        self.connect_wss(&WebsocketAPI::MultiStream.params(&endpoints.join("/")))
            .await
    }

    /// Connect to a WebSocket stream with specific market configuration
    ///
    /// # Arguments
    ///
    /// * `market` - Market type (Spot or SpotTestnet)
    /// * `subscription` - Stream subscription string
    pub async fn connect_with_market(&mut self, market: Market, subscription: &str) -> Result<()> {
        let base_url = market.base_url();
        let url = if subscription.contains('@') && !subscription.contains('?') {
            format!("{}/ws/{}", base_url, subscription)
        } else {
            if subscription.starts_with("wss://") {
                subscription.to_string()
            } else {
                format!("{}/stream?streams={}", base_url, subscription)
            }
        };
        self.connect_wss(&url).await
    }

    async fn connect_wss(&mut self, wss: &str) -> Result<()> {
        let url = Url::parse(wss)?;
        match connect_async(url).await {
            Ok((socket, _)) => {
                self.socket = Some(socket);
                Ok(())
            }
            Err(e) => Err(format!("Error during handshake {}", e).into()),
        }
    }

    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(ref mut _socket) = self.socket {
            // Note: We need to close the socket properly
            // In async context, we should use async close
            // For now, we'll just drop the socket
            self.socket = None;
            return Ok(());
        }
        Err("Not able to close the connection".into())
    }

    pub fn test_handle_msg(&mut self, msg: &str) -> Result<()> {
        self.handle_msg(msg)
    }

    pub fn handle_msg(&mut self, msg: &str) -> Result<()> {
        let value: serde_json::Value = serde_json::from_str(msg)?;

        if let Some(data) = value.get("data") {
            self.handle_msg(&data.to_string())?;
            return Ok(());
        }

        if let Ok(events) = serde_json::from_value::<Events>(value) {
            let action = match events {
                Events::DayTickerEventAll(v) => WebsocketEvent::DayTickerAll(v),
                Events::WindowTickerEventAll(v) => WebsocketEvent::WindowTickerAll(v),
                Events::BookTickerEvent(v) => WebsocketEvent::BookTicker(v),
                Events::BalanceUpdateEvent(v) => WebsocketEvent::BalanceUpdate(v),
                Events::AccountUpdateEvent(v) => WebsocketEvent::AccountUpdate(v),
                Events::OrderTradeEvent(v) => WebsocketEvent::OrderTrade(v),
                Events::AggrTradesEvent(v) => WebsocketEvent::AggrTrades(v),
                Events::TradeEvent(v) => WebsocketEvent::Trade(v),
                Events::DayTickerEvent(v) => WebsocketEvent::DayTicker(v),
                Events::WindowTickerEvent(v) => WebsocketEvent::WindowTicker(v),
                Events::KlineEvent(v) => WebsocketEvent::Kline(v),
                Events::OrderBook(v) => WebsocketEvent::OrderBook(v),
                Events::DepthOrderBookEvent(v) => WebsocketEvent::DepthOrderBook(v),
            };
            (self.handler)(action)?;
        }
        Ok(())
    }

    pub async fn event_loop(&mut self, running: &AtomicBool) -> Result<()> {
        self.event_loop_with_cancellation(Some(running), None).await
    }

    pub async fn event_loop_with_cancellation(
        &mut self, running: Option<&AtomicBool>, mut cancel_rx: Option<mpsc::UnboundedReceiver<()>>,
    ) -> Result<()> {
        let mut command_rx = self.command_rx.take().expect("command_rx already taken");
        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECONDS));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if let Some(running) = running {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
            }

            tokio::select! {
                // Handle WebSocket messages
                message = async {
                    if let Some(ref mut socket) = self.socket {
                        socket.next().await
                    } else {
                        None
                    }
                } => {
                    match message {
                        Some(Ok(message)) => match message {
                            Message::Text(msg) => {
                                if let Err(e) = self.handle_msg(&msg) {
                                    return Err(
                                        format!("Error on handling stream message: {}", e).into()
                                    );
                                }
                            }
                            Message::Ping(data) => {
                                if let Some(ref mut socket) = self.socket {
                                    let _ = socket.send(Message::Pong(data)).await;
                                }
                            }
                            Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => (),
                            Message::Close(e) => return Err(format!("Disconnected {:?}", e).into()),
                        },
                        Some(Err(e)) => return Err(format!("WebSocket error: {}", e).into()),
                        None => return Err("WebSocket stream ended".into()),
                    }
                }

                // Handle commands from command channel
                command = command_rx.recv() => {
                    match command {
                        Some(WebsocketCommand::Disconnect) => {
                            if let Some(ref mut socket) = self.socket {
                                let _ = socket.close(None).await;
                            }
                            self.socket = None;
                            break;
                        }
                        Some(WebsocketCommand::Subscribe(_stream)) => {
                            // Note: Binance WebSocket doesn't support dynamic subscription
                            // This is a placeholder for future implementation
                        }
                        Some(WebsocketCommand::Unsubscribe(_stream)) => {
                            // Note: Binance WebSocket doesn't support dynamic unsubscription
                            // This is a placeholder for future implementation
                        }
                        None => {
                            // Command channel closed
                            break;
                        }
                    }
                }

                // Handle cancellation signal
                _ = async {
                    if let Some(ref mut cancel_rx) = cancel_rx {
                        cancel_rx.recv().await
                    } else {
                        None
                    }
                } => {
                    // Cancellation signal received
                    break;
                }

                // Send periodic ping
                _ = ping_interval.tick() => {
                    if let Some(ref mut socket) = self.socket {
                        let _ = socket.send(Message::Ping(vec![])).await;
                    }
                }
            }
        }

        self.command_rx = Some(command_rx);
        Ok(())
    }

    pub fn get_command_sender(&self) -> Option<mpsc::UnboundedSender<WebsocketCommand>> {
        self.command_tx.clone()
    }

    pub fn subscribe(&self, stream: String) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(WebsocketCommand::Subscribe(stream))
                .map_err(|e| format!("Failed to send subscribe command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }

    pub fn unsubscribe(&self, stream: String) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(WebsocketCommand::Unsubscribe(stream))
                .map_err(|e| format!("Failed to send unsubscribe command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }

    pub fn disconnect_via_command(&self) -> Result<()> {
        if let Some(ref tx) = self.command_tx {
            tx.send(WebsocketCommand::Disconnect)
                .map_err(|e| format!("Failed to send disconnect command: {}", e).into())
        } else {
            Err("Command channel not available".into())
        }
    }
}
