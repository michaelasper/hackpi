/// Tokenization, preprocessing, and normalization for BM25 search.
///
/// Splits text on non-alphanumeric characters, lowercases, and filters
/// out empty or single-character tokens.
///
/// Tokenize text by splitting on non-alphanumeric characters, lowercasing,
/// and filtering out empty or single-character tokens.
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2) // skip empty and single-char tokens
        .map(|s| s.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple_words() {
        let tokens = tokenize("hello world rust code");
        assert_eq!(tokens, vec!["hello", "world", "rust", "code"]);
    }

    #[test]
    fn test_tokenize_lowercases() {
        let tokens = tokenize("Hello World Rust");
        assert_eq!(tokens, vec!["hello", "world", "rust"]);
    }

    #[test]
    fn test_tokenize_filters_short_tokens() {
        let tokens = tokenize("a an the in on at");
        // "a" is len 1, filtered. "an" is len 2, kept (our filter is len >= 2)
        // Actually the filter is s.len() >= 2, so len 2 is kept
        assert_eq!(tokens, vec!["an", "the", "in", "on", "at"]);
    }

    #[test]
    fn test_tokenize_splits_on_non_alphanumeric() {
        // Underscore is not alphanumeric, so fn_foo splits.
        // Semicolons, dots, exclamation marks, and question marks also split.
        let tokens = tokenize("fn_foo; bar.baz! qux?");
        assert_eq!(tokens, vec!["fn", "foo", "bar", "baz", "qux"]);
    }

    #[test]
    fn test_tokenize_empty_string() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_only_special_chars() {
        let tokens = tokenize("!!! ??? ...");
        assert!(tokens.is_empty());
    }
}
