use crate::config::CoreConfig;
use crate::event::{Event, RenderEvent, ReviewEvent, ScheduleEvent, SendEvent, SendPriority};
use crate::ids::derive_review_id;
use crate::state::StateView;

pub fn decide_driver_event(state: &StateView, event: &Event, _config: &CoreConfig) -> Vec<Event> {
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
            ..
        }) => {
            let group_id = state
                .sending
                .get(post_id)
                .map(|sending| sending.group_id.clone())
                .or_else(|| state.posts.get(post_id).map(|meta| meta.group_id.clone()))
                .unwrap_or_default();
            vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: *post_id,
                group_id,
                not_before_ms: *retry_at_ms,
                priority: SendPriority::Normal,
                seq: state.next_send_seq,
            })]
        }
        _ => Vec::new(),
    };

    out.extend(derived);
    out
}
