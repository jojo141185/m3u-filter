use log::{info, trace};
use tokio::task;
use shared::model::{ConfigType, PlaylistUpdateState};
use crate::api::model::{ActiveUserConnectionChangeReceiver};
use crate::api::model::{ProviderConnectionChangeReceiver};

#[derive(Clone, PartialEq)]
pub enum EventMessage {
    ServerError(String),
    ActiveUser(usize, usize), // user_count, connection count
    ActiveProvider(String, usize), // provider name, connections
    ConfigChange(ConfigType),
    PlaylistUpdate(PlaylistUpdateState),
    PlaylistUpdateProgress(String, String),
}

pub struct EventManager {
    channel_tx: tokio::sync::broadcast::Sender<EventMessage>,
    // #[allow(dead_code)]
    //channel_rx: tokio::sync::broadcast::Receiver<EventMessage>,
}

impl EventManager {
    pub fn new(mut active_user_change_rx: ActiveUserConnectionChangeReceiver,
               mut provider_change_rx: ProviderConnectionChangeReceiver,
    ) -> Self {
        let (channel_tx, _channel_rx) = tokio::sync::broadcast::channel(10);

        let channel_tx_clone = channel_tx.clone();
        task::spawn(async move {
            loop {
               tokio::select! {
                    Some((user_count, connection_count)) = active_user_change_rx.recv() => {
                        if let Err(e) = channel_tx_clone.send(EventMessage::ActiveUser(user_count, connection_count)) {
                            trace!("Failed to send active user change event: {e}");
                        }
                    }

                    Some((provider, connection_count)) = provider_change_rx.recv() => {
                        if let Err(e) = channel_tx_clone.send(EventMessage::ActiveProvider(provider, connection_count)) {
                            trace!("Failed to send active provider change event: {e}");
                        }
                    }
                    else => {
                        // Both channels are closed, exit gracefully
                        info!("All input channels closed, terminating event manager task");
                        break;
                    }
               }
           }
        });

        Self {
            channel_tx,
            //channel_rx,
        }
    }

    pub fn get_event_channel(&self) -> tokio::sync::broadcast::Receiver<EventMessage> {
        self.channel_tx.subscribe()
    }

    pub fn send_event(&self, event: EventMessage) {
        if let Err(err) = self.channel_tx.send(event) {
           trace!("Failed to send event: {err}");
        }
    }

}

