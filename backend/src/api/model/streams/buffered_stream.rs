use futures::{stream::Stream, task::{Context, Poll}, StreamExt};
use std::{
    pin::Pin,
    sync::Arc,
};
use std::cmp::{max};
use tokio::sync::mpsc::{channel, Sender};
use tokio_stream::wrappers::ReceiverStream;
use crate::api::model::BoxedProviderStream;
use crate::api::model::StreamError;
use crate::tools::atomic_once_flag::AtomicOnceFlag;

pub(in crate::api::model) struct BufferedStream {
    stream: ReceiverStream<Result<bytes::Bytes, StreamError>>,
    close_signal: Arc<AtomicOnceFlag>
}

impl BufferedStream {
    pub fn new(stream: BoxedProviderStream, buffer_size: usize, client_close_signal: Arc<AtomicOnceFlag>, _url: &str) -> Self {
        let (tx, rx) = channel(max(buffer_size, 4096));
        tokio::spawn(Self::buffer_stream(tx, stream, Arc::clone(&client_close_signal)));
        Self {
            stream: ReceiverStream::new(rx),
            close_signal: client_close_signal,
        }
    }

    async fn buffer_stream(
        tx: Sender<Result<bytes::Bytes, StreamError>>,
        mut stream: BoxedProviderStream,
        client_close_signal: Arc<AtomicOnceFlag>,
    ) {
        while client_close_signal.is_active() {
            match stream.next().await {
                Some(Ok(chunk)) => {
                  match tx.reserve().await {
                    Ok(permit) => {
                        permit.send(Ok(chunk));
                        // tokio::task::yield_now().await;
                    },
                    Err(_err) => {
                        // Receiver dropped, notify and exit
                        client_close_signal.notify();
                        break;
                    }
                  }
                }
                Some(Err(err)) => {
                    //trace!("Buffered Stream Error: {err:?}");
                    // tokio::time::sleep(sleep_duration).await;
                    // Attempt to send error to client
                    if tx.send(Err(err)).await.is_err() {
                        client_close_signal.notify();
                    }
                    break;
                }
                None => break,
            }
        }
        drop(tx);
    }
}

impl Stream for BufferedStream {
    type Item = Result<bytes::Bytes, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.close_signal.is_active() {
            Pin::new(&mut self.get_mut().stream).poll_next(cx)
        } else {
            Poll::Ready(None)
        }
    }
}
