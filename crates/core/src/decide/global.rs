use crate::command::{GlobalAction, GlobalActionCommand};
use crate::config::CoreConfig;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::event::{Event, GroupFlushReason, RenderEvent, ReviewEvent, ScheduleEvent};
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
