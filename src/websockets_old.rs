use crate::errors::Result;
use crate::config::Config;
use crate::model::{
    AccountUpdateEvent, AggrTradesEvent, BalanceUpdateEvent, BookTickerEvent, DayTickerEvent,
    WindowTickerEvent, DepthOrderBookEvent, KlineEvent, OrderBook, OrderTradeEvent, TradeEvent,
};
use url::Url;
use serde::{Deserialize, Serialize};

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
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
    pub socket: Option<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    handler: Box<dyn FnMut(WebsocketEvent) -> Result<()> + 'a>,
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
        Callback: FnMut(WebsocketEvent) -> Result<()> + 'a,
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
