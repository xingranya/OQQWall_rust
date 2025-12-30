use crate::command::{ReviewAction, ReviewActionCommand};
use crate::config::CoreConfig;
use crate::decide::builder::build_draft_from_messages;
use crate::decide::scheduler::{compute_not_before, day_index, minute_of_day};
use crate::event::{
    DraftEvent, Event, RenderEvent, RenderFormat, ReviewDecision, ReviewEvent, ScheduleEvent,
    SendPriority,
};
use crate::ids::ReviewId;
use crate::state::StateView;

pub fn decide_review_action(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
) -> Vec<Event> {
    let Some(review_id) = resolve_review_id(state, cmd) else {
        return Vec::new();
    };
    let Some(review_meta) = state.reviews.get(&review_id) else {
        return Vec::new();
    };
    let post_id = review_meta.post_id;
    let group_id = state
        .posts
        .get(&post_id)
        .map(|meta| meta.group_id.clone())
        .unwrap_or_default();

    match &cmd.action {
        ReviewAction::Approve => build_approve_events(state, cmd, config, review_id, post_id, group_id),
        ReviewAction::Reject => {
            let mut events = vec![Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Rejected,
                decided_by: cmd.operator_id.clone(),
                decided_at_ms: cmd.now_ms,
            })];
            if state.send_plans.contains_key(&post_id) {
                events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            }
            events
        }
        ReviewAction::Defer { delay_ms } => vec![
            Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Deferred,
                decided_by: cmd.operator_id.clone(),
                decided_at_ms: cmd.now_ms,
            }),
            Event::Review(ReviewEvent::ReviewDelayed {
                review_id,
                not_before_ms: cmd.now_ms.saturating_add(*delay_ms),
            }),
        ],
        ReviewAction::Skip => {
            let mut events = vec![Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Skipped,
                decided_by: cmd.operator_id.clone(),
                decided_at_ms: cmd.now_ms,
            })];
            if state.send_plans.contains_key(&post_id) {
                events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            }
            events
        }
        ReviewAction::Immediate => build_immediate_events(state, cmd, config, review_id, post_id, group_id),
        ReviewAction::Refresh => {
            let mut events = Vec::new();
            if let Some(draft_event) = rebuild_draft_event(state, post_id, cmd.now_ms) {
                events.push(Event::Draft(draft_event));
            }
            events.push(Event::Review(ReviewEvent::ReviewRefreshRequested { review_id }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                format: RenderFormat::Svg,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::Rerender => vec![
            Event::Review(ReviewEvent::ReviewRerenderRequested { review_id }),
            Event::Render(RenderEvent::RenderRequested {
                post_id,
                format: RenderFormat::Svg,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }),
        ],
        ReviewAction::SelectAllMessages => {
            let mut events = Vec::new();
            if let Some(draft_event) = rebuild_draft_event(state, post_id, cmd.now_ms) {
                events.push(Event::Draft(draft_event));
            }
            events.push(Event::Review(ReviewEvent::ReviewSelectAllRequested { review_id }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                format: RenderFormat::Svg,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::ToggleAnonymous => vec![Event::Review(ReviewEvent::ReviewAnonToggled {
            review_id,
        })],
        ReviewAction::ExpandAudit => vec![Event::Review(ReviewEvent::ReviewExpandRequested {
            review_id,
        })],
        ReviewAction::Show => vec![Event::Review(ReviewEvent::ReviewDisplayRequested {
            review_id,
        })],
        ReviewAction::Comment { text } => vec![Event::Review(ReviewEvent::ReviewCommentAdded {
            review_id,
            text: text.clone(),
        })],
        ReviewAction::Reply { text } => vec![Event::Review(ReviewEvent::ReviewReplyRequested {
            review_id,
            text: text.clone(),
        })],
        ReviewAction::Blacklist { reason } => vec![Event::Review(ReviewEvent::ReviewBlacklistRequested {
            review_id,
            reason: reason.clone(),
        })],
        ReviewAction::QuickReply { key } => vec![Event::Review(
            ReviewEvent::ReviewQuickReplyRequested {
                review_id,
                key: key.clone(),
            },
        )],
    }
}

fn build_approve_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    review_id: ReviewId,
    post_id: crate::ids::PostId,
    group_id: String,
) -> Vec<Event> {
    let mut events = Vec::new();
    events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
        review_id,
        decision: ReviewDecision::Approved,
        decided_by: cmd.operator_id.clone(),
        decided_at_ms: cmd.now_ms,
    }));

    let plan = build_send_plan(state, cmd, config, post_id, group_id, SendPriority::Normal);
    if let Some(event) = plan {
        events.push(event);
    }

    events
}

fn build_immediate_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    review_id: ReviewId,
    post_id: crate::ids::PostId,
    group_id: String,
) -> Vec<Event> {
    let mut events = Vec::new();
    events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
        review_id,
        decision: ReviewDecision::Approved,
        decided_by: cmd.operator_id.clone(),
        decided_at_ms: cmd.now_ms,
    }));

    if config.group_config(&group_id).is_some() {
        events.push(Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id: group_id.clone(),
            not_before_ms: cmd.now_ms,
            priority: SendPriority::High,
            seq: state.next_send_seq,
        }));
    }
    let minute = minute_of_day(cmd.now_ms, cmd.tz_offset_minutes);
    let day = day_index(cmd.now_ms, cmd.tz_offset_minutes);
    events.push(Event::Schedule(ScheduleEvent::GroupFlushRequested {
        group_id,
        minute_of_day: minute,
        day_index: day,
        reason: crate::event::GroupFlushReason::Manual,
    }));

    events
}

fn build_send_plan(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    post_id: crate::ids::PostId,
    group_id: String,
    priority: SendPriority,
) -> Option<Event> {
    if config.group_config(&group_id).is_none() {
        return None;
    }
    let queue_depth = state
        .send_plans
        .values()
        .filter(|plan| plan.group_id == group_id)
        .count();
    let last_send_ms = state
        .group_runtime
        .get(&group_id)
        .and_then(|runtime| runtime.last_send_ms);
    let not_before_ms = compute_not_before(
        cmd.now_ms,
        None,
        config.send_windows(&group_id),
        config.min_interval_ms(&group_id),
        last_send_ms,
        queue_depth,
        config.max_queue(&group_id),
        cmd.tz_offset_minutes,
    );
    Some(Event::Schedule(ScheduleEvent::SendPlanCreated {
        post_id,
        group_id,
        not_before_ms,
        priority,
        seq: state.next_send_seq,
    }))
}

fn resolve_review_id(state: &StateView, cmd: &ReviewActionCommand) -> Option<ReviewId> {
    if let Some(review_id) = cmd.review_id {
        return Some(review_id);
    }
    let code = cmd.review_code?;
    state.review_by_code.get(&code).copied()
}

fn rebuild_draft_event(
    state: &StateView,
    post_id: crate::ids::PostId,
    now_ms: crate::ids::TimestampMs,
) -> Option<DraftEvent> {
    let post = state.posts.get(&post_id)?;
    let ingress_ids = state.post_ingress.get(&post_id)?.clone();
    let mut messages = Vec::new();
    for ingress_id in &ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    let draft = build_draft_from_messages(&messages);
    Some(DraftEvent::PostDraftCreated {
        post_id,
        session_id: post.session_id,
        group_id: post.group_id.clone(),
        ingress_ids,
        draft,
        created_at_ms: now_ms,
    })
}
