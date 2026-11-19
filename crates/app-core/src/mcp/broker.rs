// Adapted from Super Paste (MIT); see THIRD-PARTY-NOTICES.md.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use rmcp::{RoleServer, model::ResourceUpdatedNotificationParam, service::Peer};
use serde_json::{Map, Value};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use crate::clipboard::ClipboardChange;

use crate::mcp::types::{
    MCP_CLIPBOARD_EVENT_META_KEY, MCP_CLIPBOARD_RESOURCE_URI, McpChange,
};

#[derive(Clone, Debug)]
pub(crate) struct CursorState {
    pub cursor: String,
    pub timestamp: Option<f64>,
}

pub(crate) struct ChangeBroker {
    state: Mutex<CursorState>,
    watch: watch::Sender<CursorState>,
    updates: watch::Sender<Option<Arc<McpChange>>>,
    subscribers: Mutex<HashMap<String, Weak<SessionSubscriptions>>>,
}

impl ChangeBroker {
    pub fn new() -> Arc<Self> {
        let initial = CursorState {
            cursor: format!("{}:0", Uuid::new_v4()),
            timestamp: None,
        };
        let (watch, _) = watch::channel(initial.clone());
        let (updates, _) = watch::channel(None);
        Arc::new(Self {
            state: Mutex::new(initial),
            watch,
            updates,
            subscribers: Mutex::new(HashMap::new()),
        })
    }

    pub fn cursor(&self) -> String {
        self.state
            .lock()
            .expect("MCP change state poisoned")
            .cursor
            .clone()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<CursorState> {
        self.watch.subscribe()
    }

    pub fn subscribe_updates(&self) -> watch::Receiver<Option<Arc<McpChange>>> {
        self.updates.subscribe()
    }

    pub fn begin_subscription(
        &self,
        session: &SessionSubscriptions,
        peer: Peer<RoleServer>,
    ) -> bool {
        let state = self.state.lock().expect("MCP change state poisoned");
        session.begin(peer, state.cursor.clone())
    }

    pub fn validate_cursor(&self, cursor: &str) -> Result<u64, String> {
        let (instance, version) = cursor.split_once(':').ok_or_else(invalid_cursor)?;
        let current = self.state.lock().expect("MCP change state poisoned");
        let current_instance = current.cursor.split_once(':').map(|(instance, _)| instance);
        if Some(instance) != current_instance
            || version.is_empty()
            || !version.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid_cursor());
        }
        let parsed = version.parse::<u64>().map_err(|_| invalid_cursor())?;
        if version != parsed.to_string() {
            return Err(invalid_cursor());
        }
        let current_version = current
            .cursor
            .split_once(':')
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .unwrap_or_default();
        if parsed > current_version {
            return Err(invalid_cursor());
        }
        Ok(parsed)
    }

    pub fn record(&self, change: Arc<ClipboardChange>) {
        let mut state = self.state.lock().expect("MCP change state poisoned");
        let mut version = state
            .cursor
            .split_once(':')
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .unwrap_or_default();
        if version == u64::MAX {
            version = 0;
            state.cursor = format!("{}:{}", Uuid::new_v4(), version);
        } else {
            version += 1;
            let instance = state
                .cursor
                .split_once(':')
                .map(|(value, _)| value)
                .unwrap_or_default();
            state.cursor = format!("{instance}:{version}");
        }
        let event = Arc::new(McpChange { cursor: state.cursor.clone(), change });
        state.timestamp = Some(event.change.timestamp as f64);
        self.watch.send_replace(state.clone());
        self.updates.send_replace(Some(event));
    }

    pub fn add_session(&self, session: &Arc<SessionSubscriptions>) {
        self.subscribers
            .lock()
            .expect("MCP subscriptions poisoned")
            .insert(session.id.clone(), Arc::downgrade(session));
    }

    pub fn remove_session(&self, session_id: &str) {
        self.subscribers
            .lock()
            .expect("MCP subscriptions poisoned")
            .remove(session_id);
    }

    pub fn subscribed_peers(&self) -> Vec<(String, Arc<SessionSubscriptions>, Peer<RoleServer>)> {
        let candidates = {
            let mut subscribers = self.subscribers.lock().expect("MCP subscriptions poisoned");
            subscribers.retain(|_, weak| weak.strong_count() != 0);
            subscribers.iter().map(|(id, weak)| (id.clone(), weak.clone())).collect::<Vec<_>>()
        };
        // A session's destructor removes it from this map. Upgrade and drop
        // strong references only after releasing the map lock.
        candidates.into_iter().filter_map(|(id, weak)| {
            let session = weak.upgrade()?;
            if !session.is_subscribed() { return None; }
            let peer = session.peer()?;
            Some((id, session, peer))
        }).collect()
    }
}

pub(crate) struct SessionSubscriptions {
    pub id: String,
    pub lifecycle: AsyncMutex<()>,
    peer: Mutex<Option<Peer<RoleServer>>>,
    transport_session_id: Mutex<Option<String>>,
    subscription_cursor: Mutex<Option<String>>,
    subscribed: Mutex<bool>,
    cancellation: CancellationToken,
    transport_cancellation: OnceLock<CancellationToken>,
    broker: Weak<ChangeBroker>,
}

impl SessionSubscriptions {
    pub fn new(broker: &Arc<ChangeBroker>) -> Arc<Self> {
        let session = Arc::new(Self {
            id: Uuid::new_v4().to_string(),
            lifecycle: AsyncMutex::new(()),
            peer: Mutex::new(None),
            transport_session_id: Mutex::new(None),
            subscription_cursor: Mutex::new(None),
            subscribed: Mutex::new(false),
            cancellation: CancellationToken::new(),
            transport_cancellation: OnceLock::new(),
            broker: Arc::downgrade(broker),
        });
        broker.add_session(&session);
        session
    }

    pub fn bind_transport(&self, cancellation: CancellationToken) {
        // Every request in a transport session carries the same token. Bind
        // once so closure also reaches handlers first polled after DELETE.
        let _ = self.transport_cancellation.set(cancellation);
    }

    pub fn cancellation(&self) -> &CancellationToken {
        self.transport_cancellation.get().unwrap_or(&self.cancellation)
    }

    pub fn begin(&self, peer: Peer<RoleServer>, cursor: String) -> bool {
        let mut subscribed = self
            .subscribed
            .lock()
            .expect("MCP subscription state poisoned");
        let already_subscribed = *subscribed;
        *subscribed = false;
        *self.peer.lock().expect("MCP peer state poisoned") = Some(peer);
        *self
            .subscription_cursor
            .lock()
            .expect("MCP subscription cursor lock poisoned") = Some(cursor);
        *subscribed = true;
        already_subscribed
    }

    pub fn end(&self) {
        *self
            .subscribed
            .lock()
            .expect("MCP subscription state poisoned") = false;
        self.peer.lock().expect("MCP peer state poisoned").take();
        self.subscription_cursor
            .lock()
            .expect("MCP subscription cursor lock poisoned")
            .take();
    }

    pub fn set_transport_session_id(&self, session_id: Option<String>) {
        *self
            .transport_session_id
            .lock()
            .expect("MCP transport session lock poisoned") = session_id;
    }

    pub fn transport_session_id(&self) -> Option<String> {
        self.transport_session_id
            .lock()
            .expect("MCP transport session lock poisoned")
            .clone()
    }

    pub fn should_notify(&self, cursor: &str) -> bool {
        if self.cancellation().is_cancelled() { return false; }
        let Some(start_cursor) = self
            .subscription_cursor
            .lock()
            .expect("MCP subscription cursor lock poisoned")
            .clone()
        else {
            return false;
        };
        let Some((instance, version)) = cursor.split_once(':') else {
            return false;
        };
        let Some((start_instance, start_version)) = start_cursor.split_once(':') else {
            return true;
        };
        if instance != start_instance {
            return true;
        }
        match (version.parse::<u64>(), start_version.parse::<u64>()) {
            (Ok(version), Ok(start_version)) => version > start_version,
            _ => false,
        }
    }

    pub fn is_subscribed(&self) -> bool {
        !self.cancellation().is_cancelled() && *self
            .subscribed
            .lock()
            .expect("MCP subscription state poisoned")
    }

    pub fn peer(&self) -> Option<Peer<RoleServer>> {
        self.peer.lock().expect("MCP peer state poisoned").clone()
    }
}

impl Drop for SessionSubscriptions {
    fn drop(&mut self) {
        self.cancellation().cancel();
        if let Some(broker) = self.broker.upgrade() {
            broker.remove_session(&self.id);
        }
    }
}

pub(crate) fn resource_update(event: &Value) -> ResourceUpdatedNotificationParam {
    let mut params = ResourceUpdatedNotificationParam::new(MCP_CLIPBOARD_RESOURCE_URI);
    let mut meta = Map::new();
    meta.insert(MCP_CLIPBOARD_EVENT_META_KEY.to_owned(), event.clone());
    params.meta = Some(meta.into());
    params
}

pub(crate) fn invalid_cursor() -> String {
    "剪贴板 cursor 已失效，请调用 get_clipboard 获取新的 cursor".to_owned()
}
