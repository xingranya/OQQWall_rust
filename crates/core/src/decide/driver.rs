use crate::config::CoreConfig;
use crate::event::{
    Event, RenderEvent, ReviewEvent, ScheduleEvent, SendEvent, SendPriority,
};
use crate::ids::derive_review_id;
use crate::state::StateView;

pub fn decide_driver_event(state: &StateView, event: &Event, config: &CoreConfig) -> Vec<Event> {
    match event {
        Event::Render(RenderEvent::SvgReady { post_id, .. }) => {
            if !state.posts.contains_key(post_id) {
                return Vec::new();
            }
            let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
            if state.reviews.contains_key(&review_id) {
                return Vec::new();
            }
            let review_code = state.next_review_code;
            let mut events = vec![Event::Review(ReviewEvent::ReviewItemCreated {
                review_id,
                post_id: *post_id,
                review_code,
            })];
            if !config.render_png {
                events.push(Event::Review(ReviewEvent::ReviewPublishRequested { review_id }));
            }
            events
        }
        Event::Render(RenderEvent::PngReady { post_id, .. }) => {
            if !config.render_png {
                return Vec::new();
            }
            if !state.posts.contains_key(post_id) {
                return Vec::new();
            }
            let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
            let mut events = Vec::new();
            if !state.reviews.contains_key(&review_id) {
                let review_code = state.next_review_code;
                events.push(Event::Review(ReviewEvent::ReviewItemCreated {
                    review_id,
                    post_id: *post_id,
                    review_code,
                }));
            }
            let already_published = state
                .reviews
                .get(&review_id)
                .and_then(|meta| meta.audit_msg_id.as_ref())
                .is_some();
            if !already_published {
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
    }
}
