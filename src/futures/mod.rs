pub mod account;
pub mod general;
pub mod market;
pub mod model;
pub mod userstream;
pub mod websockets;
pub mod websockets_old;

pub use websockets_old::{FuturesMarket, FuturesWebsocketEvent, FuturesWebSockets};
