use std::time::{SystemTime, UNIX_EPOCH};

use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::{ActorId, Command, CoreConfig, Event, EventEnvelope, Id128, StateView};
use tokio::sync::{broadcast, mpsc};

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        eprintln!($($arg)*);
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

pub struct Engine {
    state: StateView,
    config: CoreConfig,
    cmd_rx: mpsc::Receiver<Command>,
    bus: broadcast::Sender<EventEnvelope>,
    next_event_id: u128,
    actor: ActorId,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct EngineHandle {
    pub cmd_tx: mpsc::Sender<Command>,
    bus: broadcast::Sender<EventEnvelope>,
}

impl EngineHandle {
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.bus.subscribe()
    }
}

impl Engine {
    pub fn new(config: CoreConfig) -> (Self, EngineHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(1024);
        let (bus, _) = broadcast::channel(1024);
        let handle = EngineHandle { cmd_tx, bus: bus.clone() };
        let engine = Self {
            state: StateView::default(),
            config,
            cmd_rx,
            bus,
            next_event_id: 1,
            actor: Id128(1),
        };
        debug_log!("engine init: groups={}", engine.config.groups.len());
        (engine, handle)
    }

    pub async fn run(mut self) {
        debug_log!("engine loop started");
        while let Some(cmd) = self.cmd_rx.recv().await {
            if !matches!(cmd, Command::Tick(_)) {
                debug_log!("engine cmd: {:?}", cmd);
            }
            let events = decide(&self.state, &cmd, &self.config);
            if !events.is_empty() {
                debug_log!("engine produced {} event(s)", events.len());
                for event in &events {
                    debug_log!("engine event: {:?}", event);
                }
            }
            for event in events {
                let env = self.envelope(event);
                // TODO: journal append goes here.
                self.state = self.state.reduce(&env);
                let send_result = self.bus.send(env);
                if send_result.is_err() {
                    debug_log!("engine event bus send failed");
                }
            }
        }
        debug_log!("engine loop ended");
    }

    fn envelope(&mut self, event: Event) -> EventEnvelope {
        let env = EventEnvelope {
            id: Id128(self.next_event_id),
            ts_ms: now_ms(),
            actor: self.actor,
            correlation_id: None,
            event,
        };
        self.next_event_id = self.next_event_id.saturating_add(1);
        env
    }
}

fn now_ms() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    now.as_millis() as i64
}
