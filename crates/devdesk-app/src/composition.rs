//! AppKit offsets use UTF-16; Rust string offsets use UTF-8 bytes. Expand a
//! requested range to scalar boundaries so candidate/composition queries never
//! split an emoji's surrogate pair or panic on an invalid byte boundary.
use std::ops::Range;
pub fn range(text: &str, requested: Range<usize>) -> (Range<usize>, Range<usize>) {
    let mut utf16 = 0;
    let mut start = None;
    let mut end = None;
    for (byte, character) in text.char_indices() {
        let next = utf16 + character.len_utf16();
        if start.is_none() && requested.start < next {
            start = Some((byte, utf16));
        }
        if end.is_none() && requested.end <= utf16 {
            end = Some((byte, utf16));
        }
        utf16 = next;
    }
    let start = start.unwrap_or((text.len(), utf16));
    let end = end.unwrap_or((text.len(), utf16));
    (start.0..end.0.max(start.0), start.1..end.1.max(start.1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_queries_preserve_accents_and_expand_surrogate_pairs() {
        let text = "aé🦀界";
        let (bytes, utf16) = range(text, 1..2);
        assert_eq!(&text[bytes], "é");
        assert_eq!(utf16, 1..2);
        let (bytes, utf16) = range(text, 3..4);
        assert_eq!(&text[bytes], "🦀");
        assert_eq!(utf16, 2..4);
        let (bytes, utf16) = range(text, 99..100);
        assert_eq!(&text[bytes], "");
        assert_eq!(utf16, 5..5);
    }
}
