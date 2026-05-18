/// Code-aware chunking for source files.
///
/// Chunks source files at structural boundaries (fn, struct, impl, enum, mod, etc.)
/// using regex-based detection. This is Phase 1 — a future Phase 2 may use tree-sitter.
use std::path::Path;
use std::path::PathBuf;

/// A single chunk extracted from a source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Path to the source file this chunk belongs to.
    pub path: PathBuf,
    /// 1-based start line in the original file.
    pub start_line: usize,
    /// 1-based end line (inclusive) in the original file.
    pub end_line: usize,
    /// The structural type of this chunk (e.g., "fn", "struct", "impl", "enum", "mod", "const", "type", "file").
    pub chunk_type: String,
    /// The name of the item (e.g., function name, struct name). Empty for "file" chunks.
    pub name: String,
    /// The source text of this chunk.
    pub content: String,
}

/// Trait for chunking source files into structural units.
pub trait CodeChunker {
    /// Split `content` from `path` into chunks at structural boundaries.
    fn chunk_file(&self, path: &Path, content: &str) -> Vec<Chunk>;
}

/// A regex-based chunker for Rust source files.
///
/// Detects top-level items: functions, structs, enums, traits, unions,
/// impl blocks, modules, constants, and type aliases.
pub struct RustChunker {
    _private: (), // force construction via ::new()
}

impl RustChunker {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for RustChunker {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeChunker for RustChunker {
    fn chunk_file(&self, path: &Path, content: &str) -> Vec<Chunk> {
        if content.is_empty() {
            return Vec::new();
        }

        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        // Compile regex patterns for item detection
        let item_patterns = [
            (r"^\s*(pub\s+)?(async\s+)?(unsafe\s+)?fn\s+(\w+)", "fn"),
            (r"^\s*(pub\s+)?struct\s+(\w+)", "struct"),
            (r"^\s*(pub\s+)?enum\s+(\w+)", "enum"),
            (r"^\s*(pub\s+)?trait\s+(\w+)", "trait"),
            (r"^\s*(pub\s+)?union\s+(\w+)", "union"),
            (r"^\s*(pub\s+)?mod\s+(\w+)", "mod"),
            (r"^\s*(pub\s+)?const\s+(\w+)", "const"),
            (r"^\s*(pub\s+)?type\s+(\w+)", "type"),
            (r"^\s*impl\s*(<[^>]*>)?\s+\w+", "impl"),
        ];

        let compiled: Vec<(regex::Regex, &str)> = item_patterns
            .iter()
            .filter_map(|(pat, typ)| regex::Regex::new(pat).ok().map(|r| (r, *typ)))
            .collect();

        // Scan for top-level boundaries by tracking brace depth.
        // Only items at depth 0 (before processing the line's braces) are top-level.
        struct ItemBoundary {
            line_idx: usize,
            name: String,
            chunk_type: String,
        }

        let mut boundaries: Vec<ItemBoundary> = Vec::new();
        let mut brace_depth: i32 = 0;

        for (line_idx, line) in lines.iter().enumerate() {
            // Detect items at depth 0 (before this line's braces are counted)
            if brace_depth == 0 {
                for (re, typ) in &compiled {
                    if let Some(caps) = re.captures(line) {
                        let name = caps
                            .iter()
                            .filter_map(|m| m.map(|m| m.as_str().to_string()))
                            .fold(None::<String>, |_, v| Some(v))
                            .unwrap_or_default();
                        boundaries.push(ItemBoundary {
                            line_idx,
                            name,
                            chunk_type: typ.to_string(),
                        });
                        break;
                    }
                }
            }

            // Count braces on this line to track depth
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }
        }

        // Now extract chunk content using brace counting for each top-level boundary.
        // We skip nested boundaries (those whose end falls within a previously-extracted chunk).
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut last_end: Option<usize> = None;

        for boundary in &boundaries {
            let start_line = boundary.line_idx;

            // Skip if this boundary starts inside a previously extracted chunk
            if let Some(end) = last_end {
                if start_line <= end {
                    continue;
                }
            }

            let end_line = find_chunk_end(&lines, start_line);

            let chunk_lines: Vec<&str> = lines[start_line..=end_line].to_vec();
            let chunk_content = chunk_lines.join("\n");

            last_end = Some(end_line);

            chunks.push(Chunk {
                path: path.to_path_buf(),
                start_line: start_line + 1, // convert to 1-based
                end_line: end_line + 1,
                chunk_type: boundary.chunk_type.clone(),
                name: boundary.name.clone(),
                content: chunk_content,
            });
        }

        chunks
    }
}

/// Find the end line of a chunk starting at `start_line`.
///
/// If the start line contains `{`, uses brace counting to find the matching `}`.
/// Otherwise, looks ahead a few lines for a `{` (for items like `type` or `const`
/// where the body may start on the next line), or just takes a single line.
fn find_chunk_end(lines: &[&str], start: usize) -> usize {
    let start_line = lines[start];

    if start_line.contains('{') {
        // Brace counting from this line
        let mut depth: i32 = 0;
        let mut found_open = false;

        for (i, line) in lines.iter().enumerate().skip(start) {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        found_open = true;
                    }
                    '}' => {
                        depth -= 1;
                        if found_open && depth == 0 {
                            return i;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Unclosed brace — return last line
        return lines.len() - 1;
    }

    // No brace on start line — look ahead for one
    let lookahead_limit = std::cmp::min(start + 5, lines.len());
    for lookahead in (start + 1)..lookahead_limit {
        let la_line = lines[lookahead];
        if la_line.contains('{') {
            // Found a brace — find its matching close
            return find_chunk_end(lines, lookahead);
        }
        // If we hit a blank line or another item start, stop here
        if la_line.trim().is_empty() {
            return lookahead.saturating_sub(1);
        }
    }

    // Single-line item
    start
}

/// A fallback chunker that uses indentation-based heuristics for non-Rust files.
pub struct GenericChunker {
    _private: (), // force construction via ::new()
}

impl GenericChunker {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for GenericChunker {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeChunker for GenericChunker {
    fn chunk_file(&self, path: &Path, content: &str) -> Vec<Chunk> {
        if content.is_empty() {
            return Vec::new();
        }

        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        let mut chunks: Vec<Chunk> = Vec::new();
        let mut chunk_start = 0;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Detect top-level declarations by indentation (0 indentation for top-level)
            // and keywords like "function", "def", "class", etc.
            if !trimmed.is_empty()
                && !line.starts_with(' ')
                && !line.starts_with('\t')
                && !trimmed.starts_with("//")
                && !trimmed.starts_with("/*")
                && !trimmed.starts_with('*')
                && !trimmed.starts_with('#')
                && (trimmed.contains('(')
                    || trimmed.starts_with("export")
                    || trimmed.ends_with('{')
                    || trimmed.ends_with(";"))
            {
                if i > chunk_start {
                    let prev_lines: Vec<&str> = lines[chunk_start..i].to_vec();
                    let prev_content = prev_lines.join("\n");
                    if !prev_content.trim().is_empty() {
                        chunks.push(Chunk {
                            path: path.to_path_buf(),
                            start_line: chunk_start + 1,
                            end_line: i,
                            chunk_type: "block".to_string(),
                            name: String::new(),
                            content: prev_content,
                        });
                    }
                }
                chunk_start = i;
            }
        }

        // Push remaining lines as a chunk
        if chunk_start < lines.len() {
            let remaining: Vec<&str> = lines[chunk_start..].to_vec();
            let remaining_content = remaining.join("\n");
            if !remaining_content.trim().is_empty() {
                chunks.push(Chunk {
                    path: path.to_path_buf(),
                    start_line: chunk_start + 1,
                    end_line: lines.len(),
                    chunk_type: "block".to_string(),
                    name: String::new(),
                    content: remaining_content,
                });
            }
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Chunk struct tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_chunk_struct_fields() {
        let chunk = Chunk {
            path: PathBuf::from("test.rs"),
            start_line: 1,
            end_line: 3,
            chunk_type: "fn".to_string(),
            name: "hello".to_string(),
            content: "fn hello() {\n    println!(\"hi\");\n}".to_string(),
        };
        assert_eq!(chunk.path, PathBuf::from("test.rs"));
        assert_eq!(chunk.start_line, 1);
        assert_eq!(chunk.end_line, 3);
        assert_eq!(chunk.chunk_type, "fn");
        assert_eq!(chunk.name, "hello");
        assert_eq!(chunk.content, "fn hello() {\n    println!(\"hi\");\n}");
    }

    // -----------------------------------------------------------------------
    // RustChunker tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rust_chunker_empty_content() {
        let chunker = RustChunker::new();
        let chunks = chunker.chunk_file(Path::new("test.rs"), "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_rust_chunker_single_function() {
        let chunker = RustChunker::new();
        let content = "fn hello() {\n    println!(\"hi\");\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
    }

    #[test]
    fn test_rust_chunker_public_function() {
        let chunker = RustChunker::new();
        let content = "pub fn handle() -> bool {\n    true\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].name, "handle");
    }

    #[test]
    fn test_rust_chunker_struct_detection() {
        let chunker = RustChunker::new();
        let content = "pub struct Point {\n    x: i32,\n    y: i32,\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "struct");
        assert_eq!(chunks[0].name, "Point");
    }

    #[test]
    fn test_rust_chunker_enum_detection() {
        let chunker = RustChunker::new();
        let content = "enum Color {\n    Red,\n    Blue,\n    Green,\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "enum");
        assert_eq!(chunks[0].name, "Color");
    }

    #[test]
    fn test_rust_chunker_trait_detection() {
        let chunker = RustChunker::new();
        let content = "pub trait Drawable {\n    fn draw(&self);\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "trait");
        assert_eq!(chunks[0].name, "Drawable");
    }

    #[test]
    fn test_rust_chunker_mod_detection() {
        let chunker = RustChunker::new();
        let content = "pub mod utils {\n    fn helper() {}\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "mod");
        assert_eq!(chunks[0].name, "utils");
    }

    #[test]
    fn test_rust_chunker_const_detection() {
        let chunker = RustChunker::new();
        let content = "const MAX_SIZE: usize = 1024;\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "const");
        assert_eq!(chunks[0].name, "MAX_SIZE");
    }

    #[test]
    fn test_rust_chunker_type_alias() {
        let chunker = RustChunker::new();
        let content = "pub type Result<T> = std::result::Result<T, Error>;\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "type");
        assert_eq!(chunks[0].name, "Result");
    }

    #[test]
    fn test_rust_chunker_impl_block() {
        let chunker = RustChunker::new();
        let content = "impl Display for Point {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n        Ok(())\n    }\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "impl");
    }

    #[test]
    fn test_rust_chunker_multiple_items() {
        let chunker = RustChunker::new();
        let content = "struct Point {\n    x: i32,\n}\n\nimpl Point {\n    fn new(x: i32) -> Self {\n        Point { x }\n    }\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 2);

        // First chunk should be the struct
        assert_eq!(chunks[0].chunk_type, "struct");
        assert_eq!(chunks[0].name, "Point");
        assert!(chunks[0].content.contains("struct Point"));

        // Second chunk should be the impl
        assert_eq!(chunks[1].chunk_type, "impl");
        assert!(chunks[1].content.contains("impl Point"));
    }

    #[test]
    fn test_rust_chunker_nested_braces() {
        let chunker = RustChunker::new();
        let content = "fn outer() {\n    fn inner() {\n        // nested\n    }\n    println!(\"outer\");\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        // Should only detect the outer fn as a top-level item
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "outer");
        // Content should include everything including the inner fn
        assert!(chunks[0].content.contains("fn inner()"));
    }

    #[test]
    fn test_rust_chunker_with_comments() {
        let chunker = RustChunker::new();
        let content = "/// Documentation comment\nfn documented() -> i32 {\n    42\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "documented");
        // The doc comment is on a separate line before fn - it won't be included
        // in the chunk content since our regex only matches the fn line
        // The doc comment is not included because start_line is the fn line
        assert_eq!(chunks[0].start_line, 2); // fn is on line 2
        assert_eq!(chunks[0].end_line, 4);
    }

    // -----------------------------------------------------------------------
    // GenericChunker tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_generic_chunker_empty_content() {
        let chunker = GenericChunker::new();
        let chunks = chunker.chunk_file(Path::new("test.py"), "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_generic_chunker_detects_top_level_blocks() {
        let chunker = GenericChunker::new();
        let content = "def hello():\n    print('hi')\n\ndef goodbye():\n    print('bye')\n";
        let chunks = chunker.chunk_file(Path::new("test.py"), content);
        // Should detect two function blocks
        assert!(!chunks.is_empty(), "Should find at least one block");
    }

    // -----------------------------------------------------------------------
    // Integration: chunking paths
    // -----------------------------------------------------------------------

    #[test]
    fn test_rust_chunker_preserves_path() {
        let chunker = RustChunker::new();
        let content = "fn foo() {}";
        let chunks = chunker.chunk_file(Path::new("src/lib.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path, PathBuf::from("src/lib.rs"));
    }

    // -----------------------------------------------------------------------
    // Edge case tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rust_chunker_unsafe_fn() {
        let chunker = RustChunker::new();
        let content = "pub unsafe fn dangerous() {\n    // unsafe stuff\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].name, "dangerous");
    }

    #[test]
    fn test_rust_chunker_async_fn() {
        let chunker = RustChunker::new();
        let content = "pub async fn fetch_data() -> String {\n    \"data\".to_string()\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].name, "fetch_data");
    }

    #[test]
    fn test_rust_chunker_generic_fn() {
        let chunker = RustChunker::new();
        let content = "fn identity<T>(x: T) -> T {\n    x\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].name, "identity");
    }

    #[test]
    fn test_rust_chunker_tuple_struct() {
        let chunker = RustChunker::new();
        let content = "struct Point(i32, i32);\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "struct");
        assert_eq!(chunks[0].name, "Point");
    }

    #[test]
    fn test_rust_chunker_unit_struct() {
        let chunker = RustChunker::new();
        let content = "struct Unit;\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "struct");
        assert_eq!(chunks[0].name, "Unit");
    }

    #[test]
    fn test_rust_chunker_union_type() {
        let chunker = RustChunker::new();
        let content = "pub union IntOrFloat {\n    i: i32,\n    f: f32,\n}\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "union");
        assert_eq!(chunks[0].name, "IntOrFloat");
    }

    #[test]
    fn test_rust_chunker_no_items() {
        let chunker = RustChunker::new();
        let content = "// Just a comment\n// Another comment\n";
        let chunks = chunker.chunk_file(Path::new("test.rs"), content);
        assert!(
            chunks.is_empty(),
            "Comments-only file should produce no chunks"
        );
    }
}
