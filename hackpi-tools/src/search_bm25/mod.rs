//! BM25 full-text search index for codebase retrieval.
//!
//! Provides an in-memory BM25 ranking index over text documents and code chunks,
//! with support for cache invalidation via file modification timestamps.
//!
//! # Sub-modules
//!
//! - [`index`] — Index building, management, and document storage ([`Bm25Index`], [`Bm25Result`])
//! - [`scoring`] — BM25 scoring constants (`K1`, `B`)
//! - [`tokenizer`] — Text tokenization and normalization
//! - [`query`] — Query parsing (reserved for future expansion)

pub mod index;
pub mod query;
pub mod scoring;
pub mod tokenizer;

pub use index::{Bm25Index, Bm25Result};
