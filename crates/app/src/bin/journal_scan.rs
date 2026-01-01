use std::collections::HashMap;
use std::env;

use oqqwall_rust_core::event::{Event, IngressEvent, InputStatusKind};
use oqqwall_rust_infra::LocalJournal;

fn main() {
    let data_dir = env::args().nth(1).unwrap_or_else(|| "data".to_string());
    let journal = match LocalJournal::open(&data_dir) {
        Ok(journal) => journal,
        Err(err) => {
            eprintln!("journal open failed: {}", err);
            std::process::exit(1);
        }
    };

    let mut count: u64 = 0;
    let mut stopped_hits: u64 = 0;
    let mut last_stopped: HashMap<(String, String, String), i64> = HashMap::new();
    let replay = journal.replay(None, |env| {
        match &env.event {
            Event::Ingress(IngressEvent::InputStatusUpdated {
                profile_id: _,
                chat_id,
                user_id,
                group_id,
                status,
                received_at_ms,
            }) => {
                count = count.saturating_add(1);
                println!(
                    "input_status ts_ms={} group={} chat={} user={} status={:?} event_id={}",
                    received_at_ms, group_id, chat_id, user_id, status, env.id.0
                );
                if matches!(status, InputStatusKind::Stopped | InputStatusKind::Unknown(2)) {
                    last_stopped.insert(
                        (group_id.clone(), chat_id.clone(), user_id.clone()),
                        *received_at_ms,
                    );
                }
            }
            Event::Ingress(IngressEvent::MessageAccepted {
                group_id,
                chat_id,
                user_id,
                received_at_ms,
                ..
            })
            | Event::Ingress(IngressEvent::MessageSynced {
                group_id,
                chat_id,
                user_id,
                received_at_ms,
                ..
            }) => {
                let key = (group_id.clone(), chat_id.clone(), user_id.clone());
                if let Some(status_ts) = last_stopped.remove(&key) {
                    let delta_ms = received_at_ms.saturating_sub(status_ts);
                    stopped_hits = stopped_hits.saturating_add(1);
                    println!(
                        "input_stopped_to_message delta_ms={} status_ts={} msg_ts={} group={} chat={} user={}",
                        delta_ms, status_ts, received_at_ms, group_id, chat_id, user_id
                    );
                }
            }
            _ => {}
        }
    });

    if let Err(err) = replay {
        eprintln!("journal replay failed: {}", err);
        std::process::exit(1);
    }
    println!("input_status_total={}", count);
    println!("input_stopped_to_message_total={}", stopped_hits);
}
