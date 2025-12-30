use std::collections::HashMap;

use crate::event::{
    AccountEvent, BlobEvent, ConfigEvent, DraftEvent, Event, EventEnvelope, GroupFlushReason,
    IngressEvent, ManualEvent, MediaEvent, RenderEvent, ReviewEvent, ScheduleEvent, SendEvent,
    SessionEvent,
};
use crate::state::{
    AccountRuntime, BlobMeta, GroupRuntime, PostMeta, PostStage, RenderMeta, ReviewMeta, SendDueKey,
    SendPlan, SendingMeta, SessionKey, SessionMeta, StateView,
};

pub fn reduce(state: &StateView, env: &EventEnvelope) -> StateView {
    let mut next = state.clone();
    reduce_in_place(&mut next, env);
    next
}

fn reduce_in_place(state: &mut StateView, env: &EventEnvelope) {
    state.last_event_id = Some(env.id);
    state.last_ts_ms = Some(env.ts_ms);

    match &env.event {
        Event::System(_) => {}
        Event::Config(ConfigEvent::Applied { version, .. }) => {
            state.config_version = Some(*version);
        }
        Event::Ingress(event) => reduce_ingress(state, event),
        Event::Session(event) => reduce_session(state, event),
        Event::Draft(event) => reduce_draft(state, event),
        Event::Media(event) => reduce_media(state, event),
        Event::Render(event) => reduce_render(state, event),
        Event::Review(event) => reduce_review(state, event),
        Event::Schedule(event) => reduce_schedule(state, event),
        Event::Send(event) => reduce_send(state, event),
        Event::Blob(event) => reduce_blob(state, event),
        Event::Account(event) => reduce_account(state, event),
        Event::Manual(event) => reduce_manual(state, event),
    }
}

fn reduce_ingress(state: &mut StateView, event: &IngressEvent) {
    match event {
        IngressEvent::MessageAccepted {
            ingress_id,
            profile_id,
            chat_id,
            user_id,
            group_id,
            platform_msg_id,
            received_at_ms,
            message,
        } => {
            state.ingress_seen.insert(*ingress_id);
            state.ingress_meta.insert(
                *ingress_id,
                crate::state::IngressMeta {
                    profile_id: profile_id.clone(),
                    chat_id: chat_id.clone(),
                    user_id: user_id.clone(),
                    group_id: group_id.clone(),
                    platform_msg_id: platform_msg_id.clone(),
                    received_at_ms: *received_at_ms,
                },
            );
            state.ingress_messages.insert(*ingress_id, message.clone());
        }
        IngressEvent::MessageIgnored { ingress_id, .. } => {
            state.ingress_seen.insert(*ingress_id);
        }
    }
}

fn reduce_session(state: &mut StateView, event: &SessionEvent) {
    match event {
        SessionEvent::Opened {
            session_id,
            first_ingress_id,
            chat_id,
            user_id,
            group_id,
            close_at_ms,
        } => {
            let key = SessionKey {
                chat_id: chat_id.clone(),
                user_id: user_id.clone(),
                group_id: group_id.clone(),
            };
            let meta = SessionMeta {
                session_id: *session_id,
                key: key.clone(),
                first_ingress_id: *first_ingress_id,
                last_ingress_id: *first_ingress_id,
                close_at_ms: *close_at_ms,
            };
            state.session_by_key.insert(key, *session_id);
            state.sessions.insert(*session_id, meta);
            state
                .session_ingress
                .insert(*session_id, vec![*first_ingress_id]);
        }
        SessionEvent::Appended {
            session_id,
            ingress_id,
            close_at_ms,
        } => {
            if let Some(meta) = state.sessions.get_mut(session_id) {
                meta.last_ingress_id = *ingress_id;
                meta.close_at_ms = *close_at_ms;
            }
            state
                .session_ingress
                .entry(*session_id)
                .or_default()
                .push(*ingress_id);
        }
        SessionEvent::Closed { session_id, .. } => {
            if let Some(meta) = state.sessions.remove(session_id) {
                state.session_by_key.remove(&meta.key);
            }
            state.session_ingress.remove(session_id);
        }
    }
}

fn reduce_draft(state: &mut StateView, event: &DraftEvent) {
    match event {
        DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id,
            ingress_ids,
            draft,
            created_at_ms,
        } => {
            state.drafts.insert(*post_id, draft.clone());
            state.post_ingress.insert(*post_id, ingress_ids.clone());
            let meta = state.posts.entry(*post_id).or_insert(PostMeta {
                post_id: *post_id,
                session_id: *session_id,
                group_id: group_id.clone(),
                stage: PostStage::Drafted,
                review_id: None,
                created_at_ms: *created_at_ms,
                last_error: None,
            });
            meta.session_id = *session_id;
            meta.group_id = group_id.clone();
            if meta.created_at_ms == 0 {
                meta.created_at_ms = *created_at_ms;
            }
            state.update_post_stage(*post_id, PostStage::Drafted);
        }
    }
}

fn reduce_media(state: &mut StateView, event: &MediaEvent) {
    match event {
        MediaEvent::MediaFetchSucceeded {
            ingress_id,
            attachment_index,
            blob_id,
        } => {
            state.register_media_reference(*ingress_id, *attachment_index, *blob_id);
        }
        MediaEvent::MediaFetchFailed { .. } | MediaEvent::MediaFetchRequested { .. } => {}
    }
}

fn reduce_render(state: &mut StateView, event: &RenderEvent) {
    match event {
        RenderEvent::RenderRequested { post_id, format, .. } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                svg_blob: None,
                png_blob: None,
                last_error: None,
                last_format: None,
            });
            meta.last_format = Some(*format);
            state.update_post_stage(*post_id, PostStage::RenderRequested);
        }
        RenderEvent::SvgReady { post_id, blob_id } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                svg_blob: None,
                png_blob: None,
                last_error: None,
                last_format: None,
            });
            meta.svg_blob = Some(*blob_id);
            meta.last_error = None;
            state.update_post_stage(*post_id, PostStage::Rendered);
        }
        RenderEvent::PngReady { post_id, blob_id } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                svg_blob: None,
                png_blob: None,
                last_error: None,
                last_format: None,
            });
            meta.png_blob = Some(*blob_id);
            meta.last_error = None;
            state.update_post_stage(*post_id, PostStage::Rendered);
        }
        RenderEvent::RenderFailed {
            post_id,
            format,
            error,
            ..
        } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                svg_blob: None,
                png_blob: None,
                last_error: None,
                last_format: None,
            });
            meta.last_format = Some(*format);
            meta.last_error = Some(error.clone());
            state.update_post_stage(*post_id, PostStage::Failed);
        }
    }
}

fn reduce_review(state: &mut StateView, event: &ReviewEvent) {
    match event {
        ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        } => {
            state.review_by_code.insert(*review_code, *review_id);
            state.reviews.insert(
                *review_id,
                ReviewMeta {
                    review_id: *review_id,
                    post_id: *post_id,
                    review_code: *review_code,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    decided_by: None,
                    decided_at_ms: None,
                },
            );
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.review_id = Some(*review_id);
            }
            if state.next_review_code <= *review_code {
                state.next_review_code = review_code.saturating_add(1);
            }
            state.update_post_stage(*post_id, PostStage::ReviewPending);
        }
        ReviewEvent::ReviewPublishRequested { review_id } => {
            let post_id = state.reviews.get(review_id).map(|meta| meta.post_id);
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.delayed_until_ms = None;
            }
            if let Some(post_id) = post_id {
                state.update_post_stage(post_id, PostStage::ReviewPending);
            }
        }
        ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id,
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.audit_msg_id = Some(audit_msg_id.clone());
                state
                    .review_by_audit_msg
                    .insert(audit_msg_id.clone(), *review_id);
            }
        }
        ReviewEvent::ReviewDelayed {
            review_id,
            not_before_ms,
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.delayed_until_ms = Some(*not_before_ms);
            }
        }
        ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            decided_by,
            decided_at_ms,
        } => {
            let post_id = state.reviews.get(review_id).map(|meta| meta.post_id);
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.decision = Some(*decision);
                meta.decided_by = Some(decided_by.clone());
                meta.decided_at_ms = Some(*decided_at_ms);
            }
            if let Some(post_id) = post_id {
                let stage = match decision {
                    crate::event::ReviewDecision::Approved => PostStage::Reviewed,
                    crate::event::ReviewDecision::Rejected => PostStage::Rejected,
                    crate::event::ReviewDecision::Deferred => PostStage::ReviewPending,
                    crate::event::ReviewDecision::Skipped => PostStage::Skipped,
                };
                state.update_post_stage(post_id, stage);
            }
        }
        ReviewEvent::ReviewCommentAdded { .. }
        | ReviewEvent::ReviewReplyRequested { .. }
        | ReviewEvent::ReviewRefreshRequested { .. }
        | ReviewEvent::ReviewRerenderRequested { .. }
        | ReviewEvent::ReviewSelectAllRequested { .. }
        | ReviewEvent::ReviewBlacklistRequested { .. } => {}
    }
}

fn reduce_schedule(state: &mut StateView, event: &ScheduleEvent) {
    match event {
        ScheduleEvent::SendPlanCreated {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }
        | ScheduleEvent::SendPlanRescheduled {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
            let plan = SendPlan {
                post_id: *post_id,
                group_id: group_id.clone(),
                not_before_ms: *not_before_ms,
                priority: *priority,
                seq: *seq,
            };
            state.send_plans.insert(*post_id, plan.clone());
            state.send_due.insert(SendDueKey {
                not_before_ms: plan.not_before_ms,
                priority: plan.priority,
                seq: plan.seq,
                post_id: plan.post_id,
            });
            state.update_post_stage(*post_id, PostStage::Scheduled);
            if state.next_send_seq <= *seq {
                state.next_send_seq = seq.saturating_add(1);
            }
        }
        ScheduleEvent::SendPlanCanceled { post_id } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
        }
        ScheduleEvent::GroupFlushRequested {
            group_id,
            minute_of_day,
            day_index,
            reason: GroupFlushReason::Scheduled,
        }
        | ScheduleEvent::GroupFlushRequested {
            group_id,
            minute_of_day,
            day_index,
            reason: GroupFlushReason::Manual,
        } => {
            let runtime = state
                .group_runtime
                .entry(group_id.clone())
                .or_insert(GroupRuntime {
                    last_flush_mark: HashMap::new(),
                    last_send_ms: None,
                });
            runtime.last_flush_mark.insert(*minute_of_day, *day_index);
        }
    }
}

fn reduce_send(state: &mut StateView, event: &SendEvent) {
    match event {
        SendEvent::SendStarted {
            post_id,
            group_id,
            account_id,
            started_at_ms,
        } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
            state.sending.insert(
                *post_id,
                SendingMeta {
                    post_id: *post_id,
                    group_id: group_id.clone(),
                    account_id: account_id.clone(),
                    started_at_ms: *started_at_ms,
                },
            );
            state.update_post_stage(*post_id, PostStage::Sending);
            let runtime = state
                .group_runtime
                .entry(group_id.clone())
                .or_insert(GroupRuntime {
                    last_flush_mark: HashMap::new(),
                    last_send_ms: None,
                });
            runtime.last_send_ms = Some(*started_at_ms);
        }
        SendEvent::SendSucceeded {
            post_id,
            account_id,
            finished_at_ms,
            ..
        } => {
            let sending = state.sending.remove(post_id);
            state.update_post_stage(*post_id, PostStage::Sent);
            let runtime = state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: true,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
            runtime.last_send_ms = Some(*finished_at_ms);
            if let Some(sending) = sending {
                let runtime = state
                    .group_runtime
                    .entry(sending.group_id)
                    .or_insert(GroupRuntime {
                        last_flush_mark: HashMap::new(),
                        last_send_ms: None,
                    });
                runtime.last_send_ms = Some(*finished_at_ms);
            }
        }
        SendEvent::SendFailed {
            post_id,
            account_id,
            error,
            ..
        } => {
            state.sending.remove(post_id);
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = Some(error.clone());
            }
            state.update_post_stage(*post_id, PostStage::Failed);
            state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: true,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
        }
        SendEvent::SendGaveUp { post_id, reason } => {
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = Some(reason.clone());
            }
            state.update_post_stage(*post_id, PostStage::Manual);
        }
    }
}

fn reduce_blob(state: &mut StateView, event: &BlobEvent) {
    match event {
        BlobEvent::BlobRegistered { blob_id, size_bytes } => {
            state.blobs.insert(
                *blob_id,
                BlobMeta {
                    blob_id: *blob_id,
                    size_bytes: *size_bytes,
                    persisted_path: None,
                    ref_count: 1,
                },
            );
        }
        BlobEvent::BlobPersisted { blob_id, path } => {
            if let Some(meta) = state.blobs.get_mut(blob_id) {
                meta.persisted_path = Some(path.clone());
            }
        }
        BlobEvent::BlobReleased { blob_id } => {
            if let Some(meta) = state.blobs.get_mut(blob_id) {
                if meta.ref_count > 0 {
                    meta.ref_count -= 1;
                }
            }
        }
        BlobEvent::BlobGcRequested { blob_id } => {
            state.blobs.remove(blob_id);
        }
    }
}

fn reduce_account(state: &mut StateView, event: &AccountEvent) {
    match event {
        AccountEvent::AccountEnabled { account_id } => {
            let runtime = state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: true,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
            runtime.enabled = true;
        }
        AccountEvent::AccountDisabled { account_id } => {
            let runtime = state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: false,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
            runtime.enabled = false;
        }
        AccountEvent::AccountCooldownSet {
            account_id,
            cooldown_until_ms,
        } => {
            let runtime = state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: true,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
            runtime.cooldown_until_ms = Some(*cooldown_until_ms);
        }
        AccountEvent::AccountLastSendUpdated {
            account_id,
            last_send_ms,
        } => {
            let runtime = state.accounts.entry(account_id.clone()).or_insert(AccountRuntime {
                enabled: true,
                cooldown_until_ms: None,
                last_send_ms: None,
            });
            runtime.last_send_ms = Some(*last_send_ms);
        }
    }
}

fn reduce_manual(state: &mut StateView, event: &ManualEvent) {
    match event {
        ManualEvent::ManualInterventionRequired { post_id, .. } => {
            state.manual_interventions.insert(*post_id);
            state.update_post_stage(*post_id, PostStage::Manual);
        }
        ManualEvent::ManualInterventionResolved { post_id } => {
            state.manual_interventions.remove(post_id);
        }
    }
}
