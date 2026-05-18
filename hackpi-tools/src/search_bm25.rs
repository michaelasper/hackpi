use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Standard BM25 parameters
const K1: f64 = 1.5;
const B: f64 = 0.75;

/// A single BM25 search result
#[derive(Debug, Clone, PartialEq)]
pub struct Bm25Result {
    pub path: PathBuf,
    pub score: f64,
}

/// In-memory BM25 search index over text documents.
///
/// Builds term frequency / document frequency tables for BM25 scoring,
/// then ranks query results by relevance.
///
/// Tracks file modification times for cache invalidation. Call `is_stale()`
/// to check whether any indexed file has changed on disk since the index was built.
pub struct Bm25Index {
    /// term -> doc_id -> count within that doc
    term_freq: HashMap<String, HashMap<usize, u32>>,
    /// term -> number of documents containing it
    doc_freq: HashMap<String, u32>,
    /// total number of terms per document
    doc_lengths: Vec<usize>,
    /// average document length across all documents
    avg_doc_length: f64,
    /// document paths, indexed by doc_id
    doc_paths: Vec<PathBuf>,
    /// whether the index has been finalized
    built: bool,
    /// file modification timestamps for cache invalidation
    file_mtimes: HashMap<PathBuf, SystemTime>,
}

impl Bm25Index {
    /// Create a new empty BM25 index.
    pub fn new() -> Self {
        Self {
            term_freq: HashMap::new(),
            doc_freq: HashMap::new(),
            doc_lengths: Vec::new(),
            avg_doc_length: 0.0,
            doc_paths: Vec::new(),
            built: false,
            file_mtimes: HashMap::new(),
        }
    }

    /// Add a document to the index. The document is tokenized internally.
    /// Call `build()` after all documents are added to finalize the index.
    ///
    /// Also records the file's modification timestamp for cache invalidation.
    /// If the file's mtime cannot be read (e.g. in tests with virtual paths),
    /// the file is still indexed but mtime tracking is skipped for that path.
    pub fn add_document(&mut self, path: &Path, content: &str) {
        let doc_id = self.doc_paths.len();
        self.doc_paths.push(path.to_path_buf());

        // Record file modification time for cache invalidation
        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(mtime) = metadata.modified() {
                self.file_mtimes.insert(path.to_path_buf(), mtime);
            }
        }

        let tokens = tokenize(content);
        self.doc_lengths.push(tokens.len());

        // Count term frequencies for this document
        let mut local_tf: HashMap<String, u32> = HashMap::new();
        for token in &tokens {
            *local_tf.entry(token.clone()).or_insert(0) += 1;
        }

        // Update global term frequency and document frequency
        for (term, count) in local_tf {
            // tf: add this doc's count
            self.term_freq
                .entry(term.clone())
                .or_default()
                .insert(doc_id, count);
            // df: increment if this is the first time for this doc
            // Since we iterate unique terms from local_tf, each term from this doc
            // contributes exactly 1 to doc_freq
            *self.doc_freq.entry(term).or_insert(0) += 1;
        }
    }

    /// Finalize the index by computing average document length.
    /// Must be called before `search()`.
    pub fn build(&mut self) {
        let total: usize = self.doc_lengths.iter().sum();
        let count = self.doc_lengths.len();
        self.avg_doc_length = if count > 0 {
            total as f64 / count as f64
        } else {
            0.0
        };
        self.built = true;
    }

    /// Search the index with the given query and return top-K results
    /// ranked by BM25 score.
    ///
    /// Returns results sorted by score descending (highest relevance first).
    /// Returns an empty vec if the index is empty or the query yields no matches.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25Result> {
        if !self.built || self.doc_paths.is_empty() {
            return Vec::new();
        }

        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let n = self.doc_paths.len() as f64;
        let mut scores: Vec<f64> = vec![0.0; self.doc_paths.len()];

        for token in &query_tokens {
            let df = self.doc_freq.get(token).copied().unwrap_or(0);
            if df == 0 {
                continue;
            }

            // BM25 IDF: ln(1 + (N - df + 0.5) / (df + 0.5))
            let idf = (1.0 + (n - df as f64 + 0.5) / (df as f64 + 0.5)).ln();

            if let Some(doc_map) = self.term_freq.get(token) {
                for (doc_id, tf) in doc_map {
                    let doc_len = self.doc_lengths[*doc_id] as f64;

                    // BM25 TF component: tf * (k1 + 1) / (tf + k1 * (1 - b + b * doc_len / avg_doc_len))
                    let tf_component = if self.avg_doc_length > 0.0 {
                        let tf_float = *tf as f64;
                        tf_float * (K1 + 1.0)
                            / (tf_float + K1 * (1.0 - B + B * doc_len / self.avg_doc_length))
                    } else {
                        0.0
                    };

                    scores[*doc_id] += idf * tf_component;
                }
            }
        }

        // Build result list and sort by score descending
        let mut results: Vec<Bm25Result> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, score)| *score > 0.0)
            .map(|(doc_id, score)| Bm25Result {
                path: self.doc_paths[doc_id].clone(),
                score,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(top_k);
        results
    }

    /// Number of documents in the index.
    pub fn len(&self) -> usize {
        self.doc_paths.len()
    }

    /// Returns true if the index contains no documents.
    pub fn is_empty(&self) -> bool {
        self.doc_paths.is_empty()
    }

    /// Check whether any indexed file has been modified since the index was built.
    ///
    /// Returns `false` if no files were tracked for mtime (e.g. all files were
    /// added from in-memory content with virtual paths), or if no files have changed.
    pub fn is_stale(&self) -> bool {
        for (path, stored_mtime) in &self.file_mtimes {
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(current_mtime) = metadata.modified() {
                    if current_mtime != *stored_mtime {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize text by splitting on non-alphanumeric characters, lowercasing,
/// and filtering out empty or single-character tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2) // skip empty and single-char tokens
        .map(|s| s.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Tokenization tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Bm25Index construction tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_index_is_empty() {
        let index = Bm25Index::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_add_document_increases_count() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "fn hello() {}");
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_build_calculates_avg_doc_length() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("a.rs"), "hello world");
        index.add_document(Path::new("b.rs"), "hello rust code test");
        index.build();
        // avg = (2 + 4) / 2 = 3.0
        assert!((index.avg_doc_length - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_build_empty_index() {
        let mut index = Bm25Index::new();
        index.build();
        assert!((index.avg_doc_length - 0.0).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // Search / ranking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_search_before_build_returns_empty() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "hello world");
        let results = index.search("hello", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_index_returns_empty() {
        let mut index = Bm25Index::new();
        index.build();
        let results = index.search("hello", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_query_returns_empty() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "hello world");
        index.build();
        let results = index.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match_returns_empty() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "hello world");
        index.build();
        let results = index.search("nonexistent", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_single_document_single_match() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "hello world");
        index.build();

        let results = index.search("hello", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, Path::new("test.rs"));
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_top_k_limits_results() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("a.rs"), "hello world rust");
        index.add_document(Path::new("b.rs"), "hello world code");
        index.add_document(Path::new("c.rs"), "hello world test");
        index.build();

        let results = index.search("hello", 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_returns_top_k_sorted_by_score() {
        let mut index = Bm25Index::new();
        // doc0: contains "rust" once, "hello" once
        index.add_document(Path::new("a.rs"), "rust hello");
        // doc1: contains "rust" twice, "hello" once → higher score for "rust"
        index.add_document(Path::new("b.rs"), "rust rust hello");
        // doc2: does not contain "rust"
        index.add_document(Path::new("c.rs"), "hello world");
        index.build();

        let results = index.search("rust", 10);
        assert!(!results.is_empty());

        // doc1 (b.rs) should rank higher than doc0 (a.rs) because "rust" appears more
        let b_score = results
            .iter()
            .find(|r| r.path.ends_with("b.rs"))
            .map(|r| r.score);
        let a_score = results
            .iter()
            .find(|r| r.path.ends_with("a.rs"))
            .map(|r| r.score);

        assert!(b_score.is_some(), "b.rs should be in results");
        assert!(a_score.is_some(), "a.rs should be in results");
        assert!(
            b_score.unwrap() > a_score.unwrap(),
            "b.rs (rust×2) should score higher than a.rs (rust×1)"
        );
    }

    #[test]
    fn test_search_multi_term_query() {
        let mut index = Bm25Index::new();
        // doc0: matches both terms
        index.add_document(Path::new("full_match.rs"), "hello world rust code");
        // doc1: matches one term
        index.add_document(Path::new("partial_match.rs"), "hello goodbye");
        index.build();

        let results = index.search("hello rust", 10);
        // Both docs match "hello", but only doc0 matches "rust"
        assert_eq!(results.len(), 2);

        // full_match.rs should have a higher score
        let full = results
            .iter()
            .find(|r| r.path.ends_with("full_match.rs"))
            .map(|r| r.score);
        let partial = results
            .iter()
            .find(|r| r.path.ends_with("partial_match.rs"))
            .map(|r| r.score);
        assert!(full.unwrap() > partial.unwrap());
    }

    // -----------------------------------------------------------------------
    // BM25 scoring correctness tests
    // -----------------------------------------------------------------------

    /// Test BM25 score calculation with known values.
    ///
    /// N = 1 document
    /// doc length = 4 tokens ("hello", "world", "rust", "code")
    /// avg doc length = 4.0
    /// Query: "rust"
    ///   df("rust") = 1
    ///   idf = ln(1 + (1 - 1 + 0.5) / (1 + 0.5)) = ln(1 + 0.5/1.5) = ln(1 + 1/3) = ln(4/3) ≈ 0.28768
    ///   tf("rust") = 1
    ///   tf_component = 1 * (1.5 + 1) / (1 + 1.5 * (1 - 0.75 + 0.75 * 4/4))
    ///                = 2.5 / (1 + 1.5 * 1.0) = 2.5 / 2.5 = 1.0
    ///   score = 0.28768 * 1.0 ≈ 0.28768
    #[test]
    fn test_bm25_score_calculation_single_doc() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("test.rs"), "hello world rust code");
        index.build();

        let results = index.search("rust", 10);
        assert_eq!(results.len(), 1);

        let expected_idf = (1.0f64 + (1.0 - 1.0 + 0.5) / (1.0 + 0.5)).ln();
        let expected_score = expected_idf * 1.0; // tf_component = 1.0

        assert!(
            (results[0].score - expected_score).abs() < 1e-10,
            "Expected score {expected_score}, got {}",
            results[0].score
        );
    }

    /// Test with N=2 docs where term appears in only 1 doc.
    /// Both docs have equal length so doc_len/avg_doc_len = 1.0,
    /// which makes the TF component = 1.0 (tf=1 case).
    ///
    /// N = 2, df = 1
    /// idf = ln(1 + (2 - 1 + 0.5) / (1 + 0.5)) = ln(1 + 1.5/1.5) = ln(2) ≈ 0.693147
    /// tf_component = 1 * (1.5 + 1) / (1 + 1.5 * (1 - 0.75 + 0.75 * 1.0))
    ///              = 2.5 / (1 + 1.5 * 1.0) = 2.5 / 2.5 = 1.0
    /// score = 0.693147 * 1.0 = 0.693147
    #[test]
    fn test_bm25_idf_with_multiple_docs() {
        let mut index = Bm25Index::new();
        // Both docs have same length so avg = doc_len for both → ratio = 1.0
        index.add_document(Path::new("a.rs"), "rust is great");
        index.add_document(Path::new("b.rs"), "hiya world cool");
        index.build();

        let results = index.search("rust", 10);
        assert_eq!(results.len(), 1);

        let expected_idf = (1.0f64 + (2.0 - 1.0 + 0.5) / (1.0 + 0.5)).ln();
        let expected_score = expected_idf * 1.0; // tf_component = 1.0

        assert!(
            (results[0].score - expected_score).abs() < 1e-10,
            "Expected score {expected_score}, got {}",
            results[0].score
        );
    }

    // -----------------------------------------------------------------------
    // Cache invalidation test
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_is_reusable_for_multiple_searches() {
        let mut index = Bm25Index::new();
        index.add_document(Path::new("a.rs"), "hello world rust");
        index.add_document(Path::new("b.rs"), "hello world python");
        index.build();

        // First search
        let r1 = index.search("rust", 10);
        assert_eq!(r1.len(), 1);

        // Second search with different query
        let r2 = index.search("python", 10);
        assert_eq!(r2.len(), 1);

        // Third search with no match
        let r3 = index.search("go", 10);
        assert!(r3.is_empty());
    }

    #[test]
    fn test_default_impl() {
        let index = Bm25Index::default();
        assert!(index.is_empty());
    }

    #[test]
    fn test_search_respects_paths() {
        let mut index = Bm25Index::new();
        index.add_document(
            Path::new("src/main.rs"),
            "fn main() { println!(\"hello\"); }",
        );
        index.add_document(
            Path::new("src/lib.rs"),
            "pub fn helper() -> &str { \"world\" }",
        );
        index.build();

        let results = index.search("hello", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].path.ends_with("main.rs"));
    }

    #[test]
    fn test_doc_length_affects_score() {
        let mut index = Bm25Index::new();
        // Doc A: short, "rust" once
        index.add_document(Path::new("short.rs"), "rust");
        // Doc B: long (lots of padding), "rust" once
        index.add_document(
            Path::new("long.rs"),
            "rust something something something something something something",
        );
        index.build();

        let results = index.search("rust", 10);
        assert_eq!(results.len(), 2);

        // short.rs should score higher (same tf but shorter doc → higher tf component)
        let short = results
            .iter()
            .find(|r| r.path.ends_with("short.rs"))
            .map(|r| r.score);
        let long = results
            .iter()
            .find(|r| r.path.ends_with("long.rs"))
            .map(|r| r.score);

        assert!(
            short.unwrap() > long.unwrap(),
            "Short doc should rank higher than long doc for same term frequency"
        );
    }

    // -----------------------------------------------------------------------
    // Mtime-based cache invalidation tests
    // -----------------------------------------------------------------------

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::OnceLock<std::sync::atomic::AtomicU32> =
            std::sync::OnceLock::new();
        let c = COUNTER.get_or_init(|| std::sync::atomic::AtomicU32::new(0));
        let id = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hackpi_bm25_mtime_{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_is_stale_fresh_index_not_stale() {
        let dir = temp_dir();
        let file_path = create_file(&dir, "test.rs", "hello world rust");

        let mut index = Bm25Index::new();
        index.add_document(&file_path, "hello world rust");
        index.build();

        assert!(!index.is_stale(), "Freshly built index should not be stale");
    }

    #[test]
    fn test_is_stale_returns_true_after_file_modification() {
        let dir = temp_dir();
        let file_path = create_file(&dir, "test.rs", "hello world rust");

        let mut index = Bm25Index::new();
        index.add_document(&file_path, "hello world rust");
        index.build();

        // Before modification: not stale
        assert!(!index.is_stale(), "Index should not be stale yet");

        // Now modify the file
        std::fs::write(&file_path, "hello world rust modified").unwrap();

        // After modification: should be stale
        assert!(
            index.is_stale(),
            "Index should be stale after file modification"
        );
    }

    #[test]
    fn test_is_stale_empty_index_not_stale() {
        let index = Bm25Index::new();
        assert!(!index.is_stale(), "Empty index should not be stale");
    }

    #[test]
    fn test_is_stale_unchanged_files_not_stale() {
        let dir = temp_dir();
        let path_a = create_file(&dir, "a.rs", "hello world");
        let path_b = create_file(&dir, "b.rs", "rust code");

        let mut index = Bm25Index::new();
        index.add_document(&path_a, "hello world");
        index.add_document(&path_b, "rust code");
        index.build();

        // No modifications — should not be stale
        assert!(
            !index.is_stale(),
            "Index should not be stale when files are unchanged"
        );
    }

    #[test]
    fn test_is_stale_partial_modification_detected() {
        let dir = temp_dir();
        let path_a = create_file(&dir, "a.rs", "hello world");
        let path_b = create_file(&dir, "b.rs", "rust code");

        let mut index = Bm25Index::new();
        index.add_document(&path_a, "hello world");
        index.add_document(&path_b, "rust code");
        index.build();

        // Modify only one file
        std::fs::write(&path_a, "hello world modified").unwrap();

        assert!(
            index.is_stale(),
            "Index should be stale when any file is modified"
        );
    }

    #[test]
    fn test_is_stale_back_to_fresh_after_rebuild() {
        let dir = temp_dir();
        let file_path = create_file(&dir, "test.rs", "hello world rust");

        let mut index = Bm25Index::new();
        index.add_document(&file_path, "hello world rust");
        index.build();

        // Modify and confirm stale
        std::fs::write(&file_path, "hello world rust modified").unwrap();
        assert!(index.is_stale());

        // Rebuild the index with same content (mtime of unchanged file)
        std::fs::write(&file_path, "hello world rust modified").unwrap();
        let mut new_index = Bm25Index::new();
        new_index.add_document(&file_path, "hello world rust modified");
        new_index.build();

        assert!(
            !new_index.is_stale(),
            "After rebuild, index should not be stale"
        );
    }
}
