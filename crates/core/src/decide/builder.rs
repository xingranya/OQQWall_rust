use crate::draft::{Draft, DraftBlock, IngressMessage};

pub fn build_draft_from_messages(messages: &[IngressMessage]) -> Draft {
    let mut blocks = Vec::new();

    for message in messages {
        append_blocks_from_text(&message.text, &mut blocks);
        for attachment in &message.attachments {
            blocks.push(DraftBlock::Attachment {
                kind: attachment.kind,
                reference: attachment.reference.clone(),
            });
        }
    }

    Draft { blocks }
}

fn append_blocks_from_text(text: &str, blocks: &mut Vec<DraftBlock>) {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return;
    }
    blocks.push(DraftBlock::Paragraph {
        text: cleaned.to_string(),
    });
}
