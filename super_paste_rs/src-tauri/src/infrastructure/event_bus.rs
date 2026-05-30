use parking_lot::RwLock;
use std::collections::HashMap;
use tokio::sync::broadcast;

use crate::types::events::InternalEvent;

pub struct EventBus {
    channels: RwLock<HashMap<String, broadcast::Sender<InternalEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    pub fn publish(&self, event: InternalEvent) {
        let event_type = event.event_type();
        let channels = self.channels.read();

        if let Some(tx) = channels.get(event_type) {
            let _ = tx.send(event);
        }
    }

    pub fn subscribe(&self, event_type: &str) -> broadcast::Receiver<InternalEvent> {
        let mut channels = self.channels.write();

        channels
            .entry(event_type.to_string())
            .or_insert_with(|| broadcast::channel(100).0)
            .subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
