use crate::draft::IngressMessage;
use crate::event::Event;
use crate::ids::{GroupId, ReviewCode, ReviewId, TimestampMs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ingress(IngressCommand),
    Tick(TickCommand),
    ReviewAction(ReviewActionCommand),
    DriverEvent(Event),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressCommand {
    pub profile_id: String,
    pub chat_id: String,
    pub user_id: String,
    pub group_id: GroupId,
    pub platform_msg_id: String,
    pub message: IngressMessage,
    pub received_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickCommand {
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewActionCommand {
    pub review_id: Option<ReviewId>,
    pub review_code: Option<ReviewCode>,
    pub action: ReviewAction,
    pub operator_id: String,
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAction {
    Approve,
    Reject,
    Defer { delay_ms: TimestampMs },
    Skip,
    Immediate,
    Refresh,
    Rerender,
    SelectAllMessages,
    Comment { text: String },
    Reply { text: String },
    Blacklist { reason: Option<String> },
}
