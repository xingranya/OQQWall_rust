use crate::command::{GlobalAction, GlobalActionCommand};
use crate::config::CoreConfig;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::event::{Event, GroupFlushReason, ScheduleEvent};
use crate::state::StateView;

pub fn decide_global_action(
    state: &StateView,
    cmd: &GlobalActionCommand,
    _config: &CoreConfig,
) -> Vec<Event> {
    match &cmd.action {
        GlobalAction::SendQueueFlush => vec![Event::Schedule(ScheduleEvent::GroupFlushRequested {
            group_id: cmd.group_id.clone(),
            minute_of_day: minute_of_day(cmd.now_ms, cmd.tz_offset_minutes),
            day_index: day_index(cmd.now_ms, cmd.tz_offset_minutes),
            reason: GroupFlushReason::Manual,
        })],
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
        _ => Vec::new(),
    }
}
