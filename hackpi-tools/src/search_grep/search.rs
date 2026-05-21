use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::SearcherBuilder;
use hackpi_core::tools::ToolResult;

use crate::chunker::{CodeChunker, GenericChunker, RustChunker};
use crate::search_bm25::Bm25Index;

use super::filter::{filter_entry, load_gitignore_patterns};

pub(super) const MAX_MATCHES: usize = 50;
pub(super) const MAX_LINE_LENGTH: usize = 500;
pub(super) const DEFAULT_CONTEXT: usize = 2;
pub(super) const MAX_CONTEXT: usize = 10;

/// A (bm25_score, optional_chunk_type, optional_chunk_name) tuple.
type ScoreInfo = (f64, Option<String>, Option<String>);

/// Convert a chunk path (filepath:type:name) back to its source file path.
fn source_file_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(colon_pos) = s.rfind(':') {
        let before_colon = &s[..colon_pos];
        if let Some(second_colon) = before_colon.rfind(':') {
            return PathBuf::from(&s[..second_colon]);
        }
    }
    p.to_path_buf()
}

/// Collect per-file matches by walking the directory tree and running ripgrep.
fn collect_matches(
    workspace_root: &Path,
    matcher: &RegexMatcher,
    context_lines: usize,
    include_glob: Option<&str>,
) -> (HashMap<PathBuf, String>, bool) {
    let mut file_matches: HashMap<PathBuf, String> = HashMap::new();
    let mut match_count = 0;
    let mut truncated = false;

    let gitignore_patterns = load_gitignore_patterns(workspace_root);

    let glob_matcher = include_glob
        .and_then(|g| globset::Glob::new(g).ok())
        .map(|g| g.compile_matcher());

    let mut builder = SearcherBuilder::new();
    builder
        .line_number(true)
        .after_context(context_lines)
        .before_context(context_lines);
    let mut searcher = builder.build();

    for entry in walkdir::WalkDir::new(workspace_root)
        .into_iter()
        .filter_entry(|e| filter_entry(e, &gitignore_patterns))
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let file_path = entry.path().to_path_buf();

        // Apply glob filter against relative path
        if let Some(ref m) = glob_matcher {
            let rel_path = file_path.strip_prefix(workspace_root).unwrap_or(&file_path);
            if !m.is_match(rel_path) {
                continue;
            }
        }

        if match_count >= MAX_MATCHES {
            truncated = true;
            break;
        }

        let mut file_has_match = false;
        let mut file_output = String::new();

        let result = searcher.search_path(
            matcher,
            &file_path,
            UTF8(|lnum, line| {
                if match_count >= MAX_MATCHES {
                    return Ok(false);
                }

                if !file_has_match {
                    file_has_match = true;
                }

                let line_str = line.trim_end();
                if line_str.len() > MAX_LINE_LENGTH {
                    let msg = format!(
                        "{}:{}: [line omitted: {} chars — exceeds {} char limit]\n",
                        file_path.display(),
                        lnum,
                        line_str.len(),
                        MAX_LINE_LENGTH
                    );
                    file_output.push_str(&msg);
                    match_count += 1;
                    return Ok(true);
                }

                let msg = format!("{}:{}:  {line_str}\n", file_path.display(), lnum);
                file_output.push_str(&msg);
                match_count += 1;
                Ok(true)
            }),
        );

        if let Err(e) = result {
            tracing::warn!("Search error in {}: {e}", file_path.display());
        }

        if file_has_match {
            file_matches.insert(file_path, file_output);
        }
    }

    (file_matches, truncated)
}

/// Apply BM25 re-ranking to the collected matches.
fn apply_bm25_ranking(
    bm25: &Mutex<Option<Bm25Index>>,
    pattern: &str,
    workspace_root: &Path,
    file_matches: &mut HashMap<PathBuf, String>,
) -> Option<String> {
    let mut bm25_guard = bm25.lock().unwrap();
    let should_rebuild = match &*bm25_guard {
        Some(index) => index.is_stale() || index.is_empty(),
        None => true,
    };

    if should_rebuild {
        let mut new_index = Bm25Index::new();
        let rust_chunker = RustChunker::new();
        let generic_chunker = GenericChunker::new();

        let gitignore_patterns = load_gitignore_patterns(workspace_root);
        for entry in walkdir::WalkDir::new(workspace_root)
            .into_iter()
            .filter_entry(|e| filter_entry(e, &gitignore_patterns))
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let file_path = entry.path();

            if let Ok(content) = std::fs::read_to_string(file_path) {
                let is_rust = file_path.extension().map(|e| e == "rs").unwrap_or(false);
                if is_rust {
                    let chunks = rust_chunker.chunk_file(file_path, &content);
                    if !chunks.is_empty() {
                        new_index.add_chunked_document(file_path, &chunks);
                        continue;
                    }
                } else {
                    let chunks = generic_chunker.chunk_file(file_path, &content);
                    if !chunks.is_empty() {
                        new_index.add_chunked_document(file_path, &chunks);
                        continue;
                    }
                }
                new_index.add_document(file_path, &content);
            }
        }
        new_index.build();
        *bm25_guard = Some(new_index);
    }

    if let Some(ref index) = *bm25_guard {
        let scored = index.search(pattern, file_matches.len());

        // Build a map: source_path -> (score, chunk_type, chunk_name) for best chunk
        let mut best_chunk: HashMap<PathBuf, ScoreInfo> = HashMap::new();
        for result in &scored {
            let src = source_file_path(&result.path);
            let entry = best_chunk.entry(src).or_insert((result.score, None, None));
            if result.score > entry.0 {
                *entry = (
                    result.score,
                    result.chunk_type.clone(),
                    result.chunk_name.clone(),
                );
            }
        }

        // Sort source paths by their best chunk score
        let mut ranked_sources: Vec<(PathBuf, ScoreInfo)> = best_chunk.into_iter().collect();
        ranked_sources.sort_by(|a, b| {
            b.1 .0
                .partial_cmp(&a.1 .0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Build ordered list: scored files first by BM25 score,
        // then unscored files from file_matches
        let mut ordered: Vec<(PathBuf, String)> = Vec::new();

        // Add scored files first
        for (src_path, _) in &ranked_sources {
            if let Some(content) = file_matches.remove(src_path) {
                ordered.push((src_path.clone(), content));
            }
        }

        // Add remaining (unscored) files
        for (path, content) in file_matches.drain() {
            ordered.push((path, content));
        }

        // Format output with re-ranked order
        let mut output = String::new();
        let mut first_file = true;

        for (src_path, (_, chunk_type, chunk_name)) in &ranked_sources {
            let content = ordered
                .iter()
                .find(|(p, _)| p == src_path)
                .map(|(_, c)| c.as_str())
                .unwrap_or("");

            if !first_file {
                output.push('\n');
            }
            first_file = false;

            let header = match (chunk_type, chunk_name) {
                (Some(typ), Some(name)) if !name.is_empty() => {
                    format!("--- {}: {} {}() ---", src_path.display(), typ, name)
                }
                _ => format!("--- {} ---", src_path.display()),
            };
            output.push_str(&header);
            output.push('\n');
            output.push_str(content);
        }

        // Add remaining (unscored) files without chunk info
        for (path, content) in &ordered {
            if ranked_sources.iter().any(|(p, _)| p == path) {
                continue;
            }
            if !first_file {
                output.push('\n');
            }
            first_file = false;
            output.push_str(&format!("--- {} ---\n", path.display()));
            output.push_str(content);
        }

        if output.is_empty() {
            output = "No matches found.".to_string();
        }

        return Some(output);
    }

    None
}

/// Format matches in walk order (non-BM25 path).
fn format_walk_order(
    workspace_root: &Path,
    file_matches: &mut HashMap<PathBuf, String>,
    include_glob: Option<&str>,
    truncated: bool,
) -> String {
    let mut output = String::new();
    let mut first_file = true;

    let gitignore_patterns = load_gitignore_patterns(workspace_root);

    let glob_matcher = include_glob
        .and_then(|g| globset::Glob::new(g).ok())
        .map(|g| g.compile_matcher());

    for entry in walkdir::WalkDir::new(workspace_root)
        .into_iter()
        .filter_entry(|e| filter_entry(e, &gitignore_patterns))
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }

        // Apply glob filter against relative path
        if let Some(ref m) = glob_matcher {
            let rel_path = entry
                .path()
                .strip_prefix(workspace_root)
                .unwrap_or(entry.path());
            if !m.is_match(rel_path) {
                continue;
            }
        }

        if let Some(content) = file_matches.remove(entry.path()) {
            if !first_file {
                output.push('\n');
            }
            first_file = false;
            output.push_str(&format!("--- {} ---\n", entry.path().display()));
            output.push_str(&content);
        }
    }

    if truncated {
        output.push_str(&format!(
            "\n[Search truncated. Over {MAX_MATCHES} matches found. Refine your pattern or use include_glob.]"
        ));
    }

    if output.is_empty() {
        output = "No matches found.".to_string();
    }

    output
}

/// Execute a full grep search, optionally with BM25 re-ranking.
pub(super) fn execute_search(
    workspace_root: &Path,
    bm25: &Mutex<Option<Bm25Index>>,
    pattern: &str,
    include_glob: Option<&str>,
    context_lines: usize,
    use_bm25: bool,
) -> ToolResult {
    let matcher = match RegexMatcher::new(pattern) {
        Ok(m) => m,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Invalid regex pattern '{pattern}': {e}"),
            }
        }
    };

    let (mut file_matches, truncated) =
        collect_matches(workspace_root, &matcher, context_lines, include_glob);

    let output = if use_bm25 {
        match apply_bm25_ranking(bm25, pattern, workspace_root, &mut file_matches) {
            Some(ranked_output) => {
                // Check if we need to append truncation message
                let mut out = ranked_output;
                if truncated {
                    out.push_str(&format!(
                        "\n[Search truncated. Over {MAX_MATCHES} matches found. Refine your pattern or use include_glob.]"
                    ));
                }
                out
            }
            None => format_walk_order(workspace_root, &mut file_matches, include_glob, truncated),
        }
    } else {
        format_walk_order(workspace_root, &mut file_matches, include_glob, truncated)
    };

    ToolResult::Success { content: output }
}
