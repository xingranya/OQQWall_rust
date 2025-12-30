use std::collections::{BTreeSet, HashMap, HashSet};

use crate::draft::{Draft, IngressMessage, MediaReference};
use crate::event::{RenderFormat, ReviewDecision, SendPriority};
use crate::ids::{
    AccountId, AuditMsgId, BlobId, EventId, GroupId, IngressId, PostId, ReviewCode, ReviewId,
    SessionId, TimestampMs,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressMeta {
    pub profile_id: String,
    pub chat_id: String,
    pub user_id: String,
    pub group_id: GroupId,
    pub platform_msg_id: String,
    pub received_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub chat_id: String,
    pub user_id: String,
    pub group_id: GroupId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub session_id: SessionId,
    pub key: SessionKey,
    pub first_ingress_id: IngressId,
    pub last_ingress_id: IngressId,
    pub close_at_ms: TimestampMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostStage {
    Drafted,
    RenderRequested,
    Rendered,
    ReviewPending,
    Reviewed,
    Scheduled,
    Sending,
    Sent,
    Rejected,
    Skipped,
    Manual,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostMeta {
    pub post_id: PostId,
    pub session_id: SessionId,
    pub group_id: GroupId,
    pub stage: PostStage,
    pub review_id: Option<ReviewId>,
    pub created_at_ms: TimestampMs,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMeta {
    pub svg_blob: Option<BlobId>,
    pub png_blob: Option<BlobId>,
    pub last_error: Option<String>,
    pub last_format: Option<RenderFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewMeta {
    pub review_id: ReviewId,
    pub post_id: PostId,
    pub review_code: ReviewCode,
    pub decision: Option<ReviewDecision>,
    pub audit_msg_id: Option<AuditMsgId>,
    pub delayed_until_ms: Option<TimestampMs>,
    pub decided_by: Option<String>,
    pub decided_at_ms: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendPlan {
    pub post_id: PostId,
    pub group_id: GroupId,
    pub not_before_ms: TimestampMs,
    pub priority: SendPriority,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendDueKey {
    pub not_before_ms: TimestampMs,
    pub priority: SendPriority,
    pub seq: u64,
    pub post_id: PostId,
}

impl Ord for SendDueKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.not_before_ms, self.priority, self.seq, self.post_id.0)
            .cmp(&(other.not_before_ms, other.priority, other.seq, other.post_id.0))
    }
}

impl PartialOrd for SendDueKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendingMeta {
    pub post_id: PostId,
    pub group_id: GroupId,
    pub account_id: AccountId,
    pub started_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRuntime {
    pub enabled: bool,
    pub cooldown_until_ms: Option<TimestampMs>,
    pub last_send_ms: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRuntime {
    pub last_flush_mark: HashMap<u16, i64>,
    pub last_send_ms: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub blob_id: BlobId,
    pub size_bytes: u64,
    pub persisted_path: Option<String>,
    pub ref_count: u64,
}

#[derive(Debug, Clone)]
pub struct StateView {
    pub last_event_id: Option<EventId>,
    pub last_ts_ms: Option<TimestampMs>,
    pub config_version: Option<u64>,

    pub ingress_seen: HashSet<IngressId>,
    pub ingress_meta: HashMap<IngressId, IngressMeta>,
    pub ingress_messages: HashMap<IngressId, IngressMessage>,

    pub sessions: HashMap<SessionId, SessionMeta>,
    pub session_by_key: HashMap<SessionKey, SessionId>,
    pub session_ingress: HashMap<SessionId, Vec<IngressId>>,

    pub drafts: HashMap<PostId, Draft>,
    pub post_ingress: HashMap<PostId, Vec<IngressId>>,
    pub render: HashMap<PostId, RenderMeta>,
    pub posts: HashMap<PostId, PostMeta>,
    pub posts_by_stage: HashMap<PostStage, HashSet<PostId>>,

    pub reviews: HashMap<ReviewId, ReviewMeta>,
    pub review_by_code: HashMap<ReviewCode, ReviewId>,
    pub review_by_audit_msg: HashMap<AuditMsgId, ReviewId>,
    pub next_review_code: ReviewCode,

    pub send_plans: HashMap<PostId, SendPlan>,
    pub send_due: BTreeSet<SendDueKey>,
    pub sending: HashMap<PostId, SendingMeta>,
    pub next_send_seq: u64,

    pub accounts: HashMap<AccountId, AccountRuntime>,
    pub group_runtime: HashMap<GroupId, GroupRuntime>,

    pub blobs: HashMap<BlobId, BlobMeta>,
    pub manual_interventions: HashSet<PostId>,
}

impl Default for StateView {
    fn default() -> Self {
        Self {
            last_event_id: None,
            last_ts_ms: None,
            config_version: None,
            ingress_seen: HashSet::new(),
            ingress_meta: HashMap::new(),
            ingress_messages: HashMap::new(),
            sessions: HashMap::new(),
            session_by_key: HashMap::new(),
            session_ingress: HashMap::new(),
            drafts: HashMap::new(),
            post_ingress: HashMap::new(),
            render: HashMap::new(),
            posts: HashMap::new(),
            posts_by_stage: HashMap::new(),
            reviews: HashMap::new(),
            review_by_code: HashMap::new(),
            review_by_audit_msg: HashMap::new(),
            next_review_code: 1,
            send_plans: HashMap::new(),
            send_due: BTreeSet::new(),
            sending: HashMap::new(),
            next_send_seq: 1,
            accounts: HashMap::new(),
            group_runtime: HashMap::new(),
            blobs: HashMap::new(),
            manual_interventions: HashSet::new(),
        }
    }
}

impl StateView {
    pub fn reduce(&self, env: &crate::event::EventEnvelope) -> Self {
        crate::reduce::reduce(self, env)
    }

    pub fn update_post_stage(&mut self, post_id: PostId, stage: PostStage) {
        if let Some(meta) = self.posts.get_mut(&post_id) {
            let prev = meta.stage;
            if prev != stage {
                if let Some(set) = self.posts_by_stage.get_mut(&prev) {
                    set.remove(&post_id);
                }
            }
            meta.stage = stage;
        }
        self.posts_by_stage
            .entry(stage)
            .or_default()
            .insert(post_id);
    }

    pub fn register_media_reference(&mut self, ingress_id: IngressId, idx: usize, blob_id: BlobId) {
        if let Some(message) = self.ingress_messages.get_mut(&ingress_id) {
            if let Some(attachment) = message.attachments.get_mut(idx) {
                attachment.reference = MediaReference::Blob { blob_id };
            }
        }
    }
}
