use crate::errors::Result;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

/// Shared low-level WebSocket connection client.
///
/// Handles connection and disconnection (with proper Close frame).
///
/// Both spot (`WebSockets`) and futures (`FuturesWebSockets`) use this
/// internally, eliminating ~200 lines of duplicated connection/disconnect code.
pub(crate) struct WsClient {
    pub(crate) socket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

impl WsClient {
    /// Connect to a WebSocket URL.
    ///
    /// The URL is validated before connecting. Returns an error if the
    /// handshake fails.
    pub async fn connect(url: &str) -> Result<Self> {
        // Validate URL before passing raw string to connect_async
        let _parsed = url::Url::parse(url)
            .map_err(|e| crate::errors::Error::Msg(format!("Invalid URL '{}': {}", url, e)))?;

        let (socket, _) = connect_async(url)
            .await
            .map_err(|e| crate::errors::Error::Msg(format!("Error during handshake: {}", e)))?;

        Ok(Self {
            socket: Some(socket),
        })
    }

    /// Gracefully close the WebSocket connection.
    ///
    /// Sends a proper WebSocket Close frame, then drops the socket.
    /// Returns an error if there is no active connection.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut socket) = self.socket.take() {
            socket.close(None).await.map_err(|e| {
                crate::errors::Error::Msg(format!("Error closing WebSocket: {}", e))
            })?;
            Ok(())
        } else {
            Err(crate::errors::Error::Msg(
                "WebSocket is not connected".to_string(),
            ))
        }
    }
}
