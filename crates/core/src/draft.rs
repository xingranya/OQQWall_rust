use crate::ids::BlobId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub blocks: Vec<DraftBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftBlock {
    Paragraph { text: String },
    Attachment { kind: MediaKind, reference: MediaReference },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressMessage {
    pub text: String,
    pub attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressAttachment {
    pub kind: MediaKind,
    pub name: Option<String>,
    pub reference: MediaReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaReference {
    RemoteUrl { url: String },
    Blob { blob_id: BlobId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    File,
    Audio,
    Other,
}
