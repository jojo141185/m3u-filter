use crate::api::model::provider_stream_factory::ResponseStream;
use futures::{stream::Stream, task::{Context, Poll}, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    pin::Pin,
    sync::Arc,
};
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use crate::api::model::stream_error::StreamError;

pub(in crate::api::model) struct BufferedStream {
    stream: ReceiverStream<Result<bytes::Bytes, StreamError>>,
}

impl BufferedStream {
    pub fn new(stream: ResponseStream, buffer_size: usize, client_close_signal: Arc<AtomicBool>, _url: &str) -> Self {
        let (tx, rx) = channel(buffer_size);
        actix_rt::spawn(async move {
            let mut stream = stream;
            loop {
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        // this is for backpressure, we fill the buffer and wait for the receiver
                        if let Ok(permit) = tx.reserve().await {
                            permit.send(Ok(chunk));
                        } else {
                            client_close_signal.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                    Some(Err(_err)) => {}
                    None => {
                        break
                    }
                }
            }
        });

        Self {
            stream: ReceiverStream::new(rx)
        }
    }
}

impl Stream for BufferedStream {
    type Item = Result<bytes::Bytes, StreamError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_next_unpin(cx)
    }
}
