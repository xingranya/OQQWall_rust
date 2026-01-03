use crate::command::{GlobalAction, GlobalActionCommand};
use crate::config::CoreConfig;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::event::{
    Event, GroupFlushReason, RenderEvent, ReviewDecision, ReviewEvent, ScheduleEvent, SendEvent,
    SendPriority,
};
use crate::state::StateView;

pub fn decide_global_action(
    state: &StateView,
    cmd: &GlobalActionCommand,
    _config: &CoreConfig,
) -> Vec<Event> {
    match &cmd.action {
        GlobalAction::SendQueueFlush => {
            let mut events = vec![Event::Schedule(ScheduleEvent::GroupFlushRequested {
                group_id: cmd.group_id.clone(),
                minute_of_day: minute_of_day(cmd.now_ms, cmd.tz_offset_minutes),
                day_index: day_index(cmd.now_ms, cmd.tz_offset_minutes),
                reason: GroupFlushReason::Manual,
            })];
            events.extend(build_group_flush_events(state, &cmd.group_id, cmd.now_ms));
            events
        }
        GlobalAction::SendQueueClear => {
            let post_ids = state
                .send_plans
                .values()
                .filter(|plan| plan.group_id == cmd.group_id)
                .map(|plan| plan.post_id)
                .collect::<Vec<_>>();
            post_ids
                .into_iter()
                .map(|post_id| Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }))
                .collect()
        }
        GlobalAction::SendInFlightClear => {
            let mut events = Vec::new();
            let mut seq = state.next_send_seq;
            for sending in state.sending.values() {
                if sending.group_id != cmd.group_id {
                    continue;
                }
                let retry_at_ms = cmd.now_ms;
                events.push(Event::Send(SendEvent::SendFailed {
                    post_id: sending.post_id,
                    account_id: sending.account_id.clone(),
                    attempt: 0,
                    retry_at_ms,
                    error: "manual clear sending".to_string(),
                }));
                events.push(Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                    post_id: sending.post_id,
                    group_id: sending.group_id.clone(),
                    not_before_ms: retry_at_ms,
                    priority: SendPriority::Normal,
                    seq,
                }));
                seq = seq.saturating_add(1);
            }
            events
        }
        GlobalAction::PendingClear => {
            let mut events = Vec::new();
            for (review_id, review) in &state.reviews {
                let pending = matches!(
                    review.decision,
                    None | Some(ReviewDecision::Deferred)
                );
                if !pending {
                    continue;
                }
                let Some(post_meta) = state.posts.get(&review.post_id) else {
                    continue;
                };
                if post_meta.group_id != cmd.group_id {
                    continue;
                }
                events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
                    review_id: *review_id,
                    decision: ReviewDecision::Rejected,
                    decided_by: cmd.operator_id.clone(),
                    decided_at_ms: cmd.now_ms,
                }));
                if state.send_plans.contains_key(&review.post_id) {
                    events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled {
                        post_id: review.post_id,
                    }));
                }
            }
            events
        }
        GlobalAction::Recall { review_code } => {
            let Some(review_id) = state.review_by_code.get(review_code).copied() else {
                return Vec::new();
            };
            let Some(meta) = state.reviews.get(&review_id) else {
                return Vec::new();
            };
            vec![
                Event::Review(ReviewEvent::ReviewRerenderRequested { review_id }),
                Event::Render(RenderEvent::RenderRequested {
                    post_id: meta.post_id,
                    attempt: 1,
                    requested_at_ms: cmd.now_ms,
                }),
            ]
        }
        _ => Vec::new(),
    }
}
