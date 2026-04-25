use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BalanceUpdateEvent, BookTickerEvent, DayTickerEvent,
    WindowTickerEvent, DepthOrderBookEvent, KlineEvent, OrderBook, OrderTradeEvent, TradeEvent,
};
use crate::websockets::Market;
use url::Url;
use serde::{Deserialize, Serialize};

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};

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
    pub socket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    handler: Box<dyn FnMut(WebsocketEvent) -> Result<()> + 'a + Send>,
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
        WebSockets {
            socket: None,
            handler: Box::new(handler),
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
        self.handle_msg_depth(msg, 0)
    }

    fn handle_msg_depth(&mut self, msg: &str, depth: u32) -> Result<()> {
        if depth > 5 {
            return Ok(());
        }
        let value: serde_json::Value = serde_json::from_str(msg)?;

        if let Some(data) = value.get("data") {
            self.handle_msg_depth(&data.to_string(), depth + 1)?;
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
        while running.load(Ordering::Relaxed) {
            if let Some(ref mut socket) = self.socket {
                match socket.next().await {
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
            } else {
                return Err("WebSocket is not connected".into());
            }
        }
        Ok(())
    }
}
