//! Token-budgeted chunking with byte-offset provenance.
//!
//! [`estimate_tokens`] is the local estimator (chars/4); the live
//! path re-measures with `POST /v1/responses/input_tokens` before
//! sending. Chunks split on paragraph boundaries and carry byte
//! spans into the source text so citations survive summarization.

/// Local token estimate: ~4 chars/token. Conservative on purpose
/// (CJK/dense text underestimates — the live path re-measures).
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.chars().count() / 4).unwrap_or(u32::MAX)
}

/// One budgeted span of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// `{doc}#{idx:04}` — stable across runs.
    pub id: String,
    pub doc: String,
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub tokens: u32,
}

/// Split `text` into chunks of at most `chunk_tokens` (estimator
/// units) with `overlap_tokens` of trailing overlap. Paragraphs are
/// the atomic unit; a single over-long paragraph is hard-split.
pub fn chunk_text(doc: &str, text: &str, chunk_tokens: u32, overlap_tokens: u32) -> Vec<Chunk> {
    let chunk_tokens = chunk_tokens.max(1);
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    // Byte offset of each paragraph start.
    let mut offsets = Vec::with_capacity(paragraphs.len());
    let mut cursor = 0;
    for (i, paragraph) in paragraphs.iter().enumerate() {
        offsets.push(cursor);
        cursor += paragraph.len();
        if i + 1 < paragraphs.len() {
            cursor += 2; // the "\n\n" separator
        }
    }
    let mut chunks = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_tokens = 0u32;
    let flush = |current: &mut Vec<usize>, chunks: &mut Vec<Chunk>| {
        if current.is_empty() {
            return;
        }
        let first = offsets[current[0]];
        let last = current[current.len() - 1];
        let end = offsets[last] + paragraphs[last].len();
        let body = current
            .iter()
            .map(|i| paragraphs[*i])
            .collect::<Vec<_>>()
            .join("\n\n");
        chunks.push(Chunk {
            id: format!("{doc}#{}", chunks.len()),
            doc: doc.to_string(),
            tokens: estimate_tokens(&body),
            text: body,
            byte_start: first,
            byte_end: end,
        });
        current.clear();
    };
    for (i, paragraph) in paragraphs.iter().enumerate() {
        let tokens = estimate_tokens(paragraph);
        if tokens > chunk_tokens {
            flush(&mut current, &mut chunks);
            // Hard-split the long paragraph on char boundaries.
            let chars: Vec<char> = paragraph.chars().collect();
            let step = chunk_tokens as usize * 4;
            let mut start = 0;
            while start < chars.len() {
                let end = (start + step).min(chars.len());
                let piece: String = chars[start..end].iter().collect();
                let byte_start =
                    offsets[i] + paragraph.char_indices().nth(start).map_or(0, |(b, _)| b);
                chunks.push(Chunk {
                    id: format!("{doc}#{}", chunks.len()),
                    doc: doc.to_string(),
                    tokens: estimate_tokens(&piece),
                    byte_start,
                    byte_end: byte_start + piece.len(),
                    text: piece,
                });
                start = end;
            }
            current_tokens = 0;
            continue;
        }
        if !current.is_empty() && current_tokens + tokens > chunk_tokens {
            flush(&mut current, &mut chunks);
            current_tokens = 0;
        }
        current.push(i);
        current_tokens += tokens;
    }
    flush(&mut current, &mut chunks);
    apply_overlap(chunks, overlap_tokens)
}

/// Append trailing overlap from the previous chunk's tail.
fn apply_overlap(mut chunks: Vec<Chunk>, overlap_tokens: u32) -> Vec<Chunk> {
    if overlap_tokens == 0 || chunks.len() < 2 {
        return chunks;
    }
    let overlap_chars = overlap_tokens as usize * 4;
    for i in 1..chunks.len() {
        let tail: String = chunks[i - 1]
            .text
            .chars()
            .rev()
            .take(overlap_chars)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if tail.len() < chunks[i - 1].text.len() {
            chunks[i].text = format!("{tail}\n\n{}", chunks[i].text);
            chunks[i].tokens = estimate_tokens(&chunks[i].text);
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimator_scales_with_length() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn paragraphs_pack_and_offsets_hold() {
        let text = "aaa\n\nbbbb\n\ncc";
        let chunks = chunk_text("d", text, 10, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].byte_start, 0);
        assert_eq!(&text[chunks[0].byte_start..chunks[0].byte_end], text);
        // 8-char paragraphs ≈ 2 tokens each; budget 2 → one per chunk.
        let text = "aaaaaaaa\n\nbbbbbbbb\n\ncccccccc";
        let chunks = chunk_text("d", text, 2, 0);
        assert_eq!(chunks.len(), 3);
        for chunk in &chunks {
            assert_eq!(&text[chunk.byte_start..chunk.byte_end], chunk.text);
        }
    }

    #[test]
    fn long_paragraph_hard_splits() {
        let text = "x".repeat(1000);
        let chunks = chunk_text("d", &text, 100, 0);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.tokens <= 100));
    }

    #[test]
    fn overlap_carries_tail() {
        let text = "a".repeat(400) + "\n\n" + &"b".repeat(400);
        let chunks = chunk_text("d", &text, 100, 25);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].text.starts_with(&"a".repeat(100)));
    }
}
