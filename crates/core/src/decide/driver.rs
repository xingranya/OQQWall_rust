use crate::config::CoreConfig;
use crate::event::{
    Event, ManualEvent, RenderEvent, ReviewDecision, ReviewEvent, ScheduleEvent, SendEvent,
    SendPriority,
};
use crate::ids::{derive_review_id, PostId, ReviewCode, ReviewId};
use crate::state::StateView;

pub fn decide_driver_event(state: &StateView, event: &Event, config: &CoreConfig) -> Vec<Event> {
    // Driver events come from IO drivers. They must flow into the event stream
    // (reduce/broadcast), otherwise other drivers cannot observe key events like
    // BlobPersisted / RenderReady / MediaFetchSucceeded / SendSucceeded.
    // Pass through the original event and append any derived events as needed.
    let mut out = vec![event.clone()];

    let derived = match event {
        Event::Render(RenderEvent::PngReady { post_id, .. }) => {
            if !state.posts.contains_key(post_id) {
                return out;
            }
            let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
            let review_meta = state.reviews.get(&review_id);
            let needs_republish = review_meta
                .map(|meta| meta.needs_republish)
                .unwrap_or(false);
            let mut events = Vec::new();
            let review_code = review_meta
                .map(|meta| meta.review_code)
                .unwrap_or(state.next_review_code);
            if review_meta.is_none() {
                events.push(Event::Review(ReviewEvent::ReviewItemCreated {
                    review_id,
                    post_id: *post_id,
                    review_code,
                }));
            }
            let already_published = review_meta
                .and_then(|meta| meta.audit_msg_id.as_ref())
                .is_some();
            if needs_republish || !already_published {
                events.push(Event::Review(ReviewEvent::ReviewInfoSynced {
                    review_id,
                    post_id: *post_id,
                    review_code,
                }));
                events.push(Event::Review(ReviewEvent::ReviewPublishRequested { review_id }));
            }
            events
        }
        Event::Send(SendEvent::SendFailed {
            post_id,
            retry_at_ms,
            attempt,
            error,
            ..
        }) => {
            let group_id = state
                .sending
                .get(post_id)
                .map(|sending| sending.group_id.clone())
                .or_else(|| state.posts.get(post_id).map(|meta| meta.group_id.clone()))
                .unwrap_or_default();
            let decided_at_ms = state.last_ts_ms.unwrap_or(0);
            if is_send_timeout_error(error) {
                return_to_pending(state, *post_id, decided_at_ms)
            } else {
                let max_attempts = config.send_max_attempts(&group_id);
                if max_attempts > 0 && *attempt >= max_attempts {
                    let reason = format!("send failed after {} attempts: {}", attempt, error);
                    let mut events = vec![
                        Event::Send(SendEvent::SendGaveUp {
                            post_id: *post_id,
                            reason: reason.clone(),
                        }),
                        Event::Manual(ManualEvent::ManualInterventionRequired {
                            post_id: *post_id,
                            reason,
                        }),
                    ];
                    events.extend(return_to_pending(state, *post_id, decided_at_ms));
                    events
                } else {
                    vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                        post_id: *post_id,
                        group_id,
                        not_before_ms: *retry_at_ms,
                        priority: SendPriority::Normal,
                        seq: state.next_send_seq,
                    })]
                }
            }
        }
        Event::Send(SendEvent::SendGaveUp { post_id, .. }) => {
            let decided_at_ms = state.last_ts_ms.unwrap_or(0);
            return_to_pending(state, *post_id, decided_at_ms)
        }
        _ => Vec::new(),
    };

    out.extend(derived);
    out
}

fn is_send_timeout_error(error: &str) -> bool {
    error.starts_with("send timeout")
}

fn return_to_pending(state: &StateView, post_id: PostId, decided_at_ms: i64) -> Vec<Event> {
    let Some((review_id, review_code)) = resolve_review_meta(state, post_id) else {
        return Vec::new();
    };
    vec![
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision: ReviewDecision::Deferred,
            decided_by: "system".to_string(),
            decided_at_ms,
        }),
        Event::Review(ReviewEvent::ReviewInfoSynced {
            review_id,
            post_id,
            review_code,
        }),
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id }),
    ]
}

fn resolve_review_meta(state: &StateView, post_id: PostId) -> Option<(ReviewId, ReviewCode)> {
    let review_id = state
        .posts
        .get(&post_id)
        .and_then(|meta| meta.review_id)
        .or_else(|| {
            let derived = derive_review_id(&[&post_id.to_be_bytes()]);
            state.reviews.contains_key(&derived).then_some(derived)
        })?;
    let review = state.reviews.get(&review_id)?;
    Some((review_id, review.review_code))
}
