#![cfg(test)]

use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_unbounded_sender_receives_and_prints() {
    // This test demonstrates using a tokio unbounded channel sender and printing
    // the received value on the receiver side.
    //
    // It spawns a task that sends a message through the UnboundedSender and
    // awaits the receiver. The received value is printed and asserted.

    use tokio::sync::mpsc;

    // Create an unbounded channel
    let (tx, mut rx): (
        mpsc::UnboundedSender<String>,
        mpsc::UnboundedReceiver<String>,
    ) = mpsc::unbounded_channel();

    // Clone the sender and spawn a sender task.
    let sender = tx.clone();
    tokio::spawn(async move {
        // Simulate some small async work before sending
        sleep(Duration::from_millis(50)).await;
        let msg = String::from("hello from unbounded");
        // UnboundedSender::send returns Result<(), T> where Err(T) indicates receiver was closed.
        sender
            .send(msg)
            .expect("failed to send on unbounded channel");
    });

    // Receive the message and print it.
    match rx.recv().await {
        Some(received) => {
            eprintln!("Received via unbounded channel: {}", received);
            assert_eq!(received, "hello from unbounded");
        }
        None => panic!("Channel closed before a message was received"),
    }
}
