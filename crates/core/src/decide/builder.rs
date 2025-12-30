use crate::draft::{Draft, DraftBlock, IngressMessage};

const SOFT_SPLIT_LIMIT: usize = 800;
const HARD_SPLIT_LIMIT: usize = 1000;

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
    let mut buf = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            flush_segment(&buf, blocks);
            buf.clear();
            continue;
        }
        buf.push(line);
    }

    flush_segment(&buf, blocks);
}

fn flush_segment(lines: &[&str], blocks: &mut Vec<DraftBlock>) {
    if lines.is_empty() {
        return;
    }

    let segment = lines.join("\n");
    let segments = split_long_segment(&segment);
    for piece in segments {
        if !piece.trim().is_empty() {
            blocks.push(DraftBlock::Paragraph { text: piece });
        }
    }
}

fn split_long_segment(segment: &str) -> Vec<String> {
    if segment.chars().count() <= SOFT_SPLIT_LIMIT {
        return vec![segment.to_string()];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for ch in segment.chars() {
        current.push(ch);
        current_len += 1;

        if is_sentence_break(ch) && current_len >= SOFT_SPLIT_LIMIT {
            push_piece(&mut pieces, &mut current, &mut current_len);
        } else if current_len >= HARD_SPLIT_LIMIT {
            push_piece(&mut pieces, &mut current, &mut current_len);
        }
    }

    if !current.is_empty() {
        pieces.push(current);
    }

    pieces
}

fn push_piece(pieces: &mut Vec<String>, current: &mut String, current_len: &mut usize) {
    if current.trim().is_empty() {
        current.clear();
        *current_len = 0;
        return;
    }
    pieces.push(current.clone());
    current.clear();
    *current_len = 0;
}

fn is_sentence_break(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ';' | '\u{3002}' | '\u{FF01}' | '\u{FF1F}' | '\u{FF1B}'
    )
}
