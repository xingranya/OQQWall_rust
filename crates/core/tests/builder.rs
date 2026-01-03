use oqqwall_rust_core::{
    build_draft_from_messages, DraftBlock, IngressAttachment, IngressMessage, MediaKind,
    MediaReference,
};

#[test]
fn build_draft_splits_paragraphs_and_keeps_attachments() {
    let message = IngressMessage {
        text: "alpha\n\n beta".to_string(),
        attachments: vec![IngressAttachment {
            kind: MediaKind::Image,
            name: None,
            reference: MediaReference::RemoteUrl {
                url: "http://example.com/img.png".to_string(),
            },
            size_bytes: None,
        }],
    };

    let draft = build_draft_from_messages(&[message]);
    assert_eq!(draft.blocks.len(), 2);
    assert!(matches!(draft.blocks[0], DraftBlock::Paragraph { .. }));
    assert!(matches!(draft.blocks[1], DraftBlock::Attachment { .. }));
}
