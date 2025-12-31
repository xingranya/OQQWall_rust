use crate::draft::{Draft, IngressMessage};
use crate::ids::{
    AccountId, ActorId, AuditMsgId, BlobId, CorrelationId, EventId, GroupId, IngressId, PostId,
    RemotePostId, ReviewCode, ReviewId, SessionId, TimestampMs,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: EventId,
    pub ts_ms: TimestampMs,
    pub actor: ActorId,
    pub correlation_id: Option<CorrelationId>,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    System(SystemEvent),
    Config(ConfigEvent),
    Ingress(IngressEvent),
    Session(SessionEvent),
    Draft(DraftEvent),
    Media(MediaEvent),
    Render(RenderEvent),
    Review(ReviewEvent),
    Schedule(ScheduleEvent),
    Send(SendEvent),
    Blob(BlobEvent),
    Account(AccountEvent),
    Manual(ManualEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemEvent {
    Booted,
    SnapshotLoaded,
    SnapshotTaken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigEvent {
    Applied {
        version: u64,
        config_blob: Option<BlobId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngressEvent {
    MessageAccepted {
        ingress_id: IngressId,
        profile_id: String,
        chat_id: String,
        user_id: String,
        sender_name: Option<String>,
        group_id: GroupId,
        platform_msg_id: String,
        received_at_ms: TimestampMs,
        message: IngressMessage,
    },
    MessageIgnored {
        ingress_id: IngressId,
        reason: IngressIgnoreReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngressIgnoreReason {
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEvent {
    Opened {
        session_id: SessionId,
        first_ingress_id: IngressId,
        chat_id: String,
        user_id: String,
        group_id: GroupId,
        close_at_ms: TimestampMs,
    },
    Appended {
        session_id: SessionId,
        ingress_id: IngressId,
        close_at_ms: TimestampMs,
    },
    Closed {
        session_id: SessionId,
        closed_at_ms: TimestampMs,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftEvent {
    PostDraftCreated {
        post_id: PostId,
        session_id: SessionId,
        group_id: GroupId,
        ingress_ids: Vec<IngressId>,
        draft: Draft,
        created_at_ms: TimestampMs,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaEvent {
    MediaFetchRequested {
        ingress_id: IngressId,
        attachment_index: usize,
        attempt: u32,
    },
    MediaFetchSucceeded {
        ingress_id: IngressId,
        attachment_index: usize,
        blob_id: BlobId,
    },
    MediaFetchFailed {
        ingress_id: IngressId,
        attachment_index: usize,
        attempt: u32,
        retry_at_ms: TimestampMs,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderEvent {
    RenderRequested {
        post_id: PostId,
        attempt: u32,
        requested_at_ms: TimestampMs,
    },
    PngReady {
        post_id: PostId,
        blob_id: BlobId,
    },
    RenderFailed {
        post_id: PostId,
        attempt: u32,
        retry_at_ms: TimestampMs,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewEvent {
    ReviewItemCreated {
        review_id: ReviewId,
        post_id: PostId,
        review_code: ReviewCode,
    },
    ReviewPublishRequested {
        review_id: ReviewId,
    },
    ReviewPublished {
        review_id: ReviewId,
        audit_msg_id: AuditMsgId,
    },
    ReviewDelayed {
        review_id: ReviewId,
        not_before_ms: TimestampMs,
    },
    ReviewDecisionRecorded {
        review_id: ReviewId,
        decision: ReviewDecision,
        decided_by: String,
        decided_at_ms: TimestampMs,
    },
    ReviewCommentAdded {
        review_id: ReviewId,
        text: String,
    },
    ReviewReplyRequested {
        review_id: ReviewId,
        text: String,
    },
    ReviewRefreshRequested {
        review_id: ReviewId,
    },
    ReviewRerenderRequested {
        review_id: ReviewId,
    },
    ReviewSelectAllRequested {
        review_id: ReviewId,
    },
    ReviewAnonToggled {
        review_id: ReviewId,
    },
    ReviewExpandRequested {
        review_id: ReviewId,
    },
    ReviewDisplayRequested {
        review_id: ReviewId,
    },
    ReviewBlacklistRequested {
        review_id: ReviewId,
        reason: Option<String>,
    },
    ReviewQuickReplyRequested {
        review_id: ReviewId,
        key: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Approved,
    Rejected,
    Deferred,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SendPriority {
    High,
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleEvent {
    SendPlanCreated {
        post_id: PostId,
        group_id: GroupId,
        not_before_ms: TimestampMs,
        priority: SendPriority,
        seq: u64,
    },
    SendPlanRescheduled {
        post_id: PostId,
        group_id: GroupId,
        not_before_ms: TimestampMs,
        priority: SendPriority,
        seq: u64,
    },
    SendPlanCanceled {
        post_id: PostId,
    },
    GroupFlushRequested {
        group_id: GroupId,
        minute_of_day: u16,
        day_index: i64,
        reason: GroupFlushReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupFlushReason {
    Scheduled,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendEvent {
    SendStarted {
        post_id: PostId,
        group_id: GroupId,
        account_id: AccountId,
        started_at_ms: TimestampMs,
    },
    SendSucceeded {
        post_id: PostId,
        account_id: AccountId,
        finished_at_ms: TimestampMs,
        remote_id: Option<RemotePostId>,
    },
    SendFailed {
        post_id: PostId,
        account_id: AccountId,
        attempt: u32,
        retry_at_ms: TimestampMs,
        error: String,
    },
    SendGaveUp {
        post_id: PostId,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlobEvent {
    BlobRegistered {
        blob_id: BlobId,
        size_bytes: u64,
    },
    BlobPersisted {
        blob_id: BlobId,
        path: String,
    },
    BlobReleased {
        blob_id: BlobId,
    },
    BlobGcRequested {
        blob_id: BlobId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountEvent {
    AccountEnabled {
        account_id: AccountId,
    },
    AccountDisabled {
        account_id: AccountId,
    },
    AccountCooldownSet {
        account_id: AccountId,
        cooldown_until_ms: TimestampMs,
    },
    AccountLastSendUpdated {
        account_id: AccountId,
        last_send_ms: TimestampMs,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManualEvent {
    ManualInterventionRequired {
        post_id: PostId,
        reason: String,
    },
    ManualInterventionResolved {
        post_id: PostId,
    },
}
