use crate::command::TickCommand;
use crate::config::CoreConfig;
use crate::decide::builder::build_draft_from_messages;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::decide::sender::{choose_account, AccountChoice};
use crate::event::{
    DraftEvent, Event, GroupFlushReason, RenderEvent, ScheduleEvent, SendEvent, SendPriority,
    SessionEvent,
};
use crate::ids::derive_post_id;
use crate::state::StateView;

pub fn decide_tick(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    let mut events = Vec::new();

    events.extend(close_due_sessions(state, cmd, config));
    events.extend(trigger_review_delays(state, cmd));
    events.extend(trigger_group_flush(state, cmd, config));
    events.extend(maybe_start_send(state, cmd, config));

    events
}

fn close_due_sessions(state: &StateView, cmd: &TickCommand, _config: &CoreConfig) -> Vec<Event> {
    let mut due_sessions = state
        .sessions
        .values()
        .filter(|meta| cmd.now_ms >= meta.close_at_ms)
        .map(|meta| meta.session_id)
        .collect::<Vec<_>>();
    due_sessions.sort();

    let mut events = Vec::new();
    for session_id in due_sessions {
        let Some(session_meta) = state.sessions.get(&session_id) else {
            continue;
        };
        let Some(ingress_ids) = state.session_ingress.get(&session_id) else {
            continue;
        };
        let mut messages = Vec::new();
        for ingress_id in ingress_ids {
            if let Some(message) = state.ingress_messages.get(ingress_id) {
                messages.push(message.clone());
            }
        }
        let draft = build_draft_from_messages(&messages);
        let session_bytes = session_id.to_be_bytes();
        let post_id = derive_post_id(&[&session_bytes]);

        events.push(Event::Session(SessionEvent::Closed {
            session_id,
            closed_at_ms: cmd.now_ms,
        }));
        events.push(Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: session_meta.key.group_id.clone(),
            ingress_ids: ingress_ids.clone(),
            draft,
            created_at_ms: cmd.now_ms,
        }));
        events.push(Event::Render(RenderEvent::RenderRequested {
            post_id,
            attempt: 1,
            requested_at_ms: cmd.now_ms,
        }));
    }

    events
}

fn trigger_review_delays(state: &StateView, cmd: &TickCommand) -> Vec<Event> {
    let mut events = Vec::new();
    for review in state.reviews.values() {
        if let Some(delay_until) = review.delayed_until_ms {
            if cmd.now_ms >= delay_until {
                events.push(Event::Review(crate::event::ReviewEvent::ReviewPublishRequested {
                    review_id: review.review_id,
                }));
            }
        }
    }
    events
}

fn trigger_group_flush(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    let mut events = Vec::new();
    let minute = minute_of_day(cmd.now_ms, cmd.tz_offset_minutes);
    let day = day_index(cmd.now_ms, cmd.tz_offset_minutes);

    for (group_id, group_cfg) in &config.groups {
        if !group_cfg.send_schedule_minutes.contains(&minute) {
            continue;
        }
        let already = state
            .group_runtime
            .get(group_id)
            .and_then(|runtime| runtime.last_flush_mark.get(&minute))
            .copied();
        if already == Some(day) {
            continue;
        }
        events.push(Event::Schedule(ScheduleEvent::GroupFlushRequested {
            group_id: group_id.clone(),
            minute_of_day: minute,
            day_index: day,
            reason: GroupFlushReason::Scheduled,
        }));
        events.extend(build_group_flush_events(state, group_id, cmd.now_ms));
    }

    events
}

fn maybe_start_send(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    if !state.sending.is_empty() {
        return Vec::new();
    }

    let due = state
        .send_due
        .iter()
        .find(|key| key.not_before_ms <= cmd.now_ms)
        .copied();
    let Some(due) = due else {
        return Vec::new();
    };
    let Some(plan) = state.send_plans.get(&due.post_id) else {
        return Vec::new();
    };

    match choose_account(state, config.group_config(&plan.group_id), cmd.now_ms) {
        AccountChoice::Available(account_id) => vec![Event::Send(SendEvent::SendStarted {
            post_id: plan.post_id,
            group_id: plan.group_id.clone(),
            account_id,
            started_at_ms: cmd.now_ms,
        })],
        AccountChoice::RetryAt(retry_at) => vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id: plan.post_id,
            group_id: plan.group_id.clone(),
            not_before_ms: retry_at,
            priority: plan.priority,
            seq: state.next_send_seq,
        })],
        AccountChoice::Unavailable => vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id: plan.post_id,
            group_id: plan.group_id.clone(),
            not_before_ms: cmd.now_ms.saturating_add(30_000),
            priority: SendPriority::Normal,
            seq: state.next_send_seq,
        })],
    }
}
