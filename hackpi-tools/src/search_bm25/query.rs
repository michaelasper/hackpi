//! Query parsing and matching for BM25 search.
//!
//! This module is reserved for future query expansion features such as
//! phrase matching, boolean operators, and field-scoped queries.
//! Currently, query parsing is handled inline in the index's `search()` method
//! via the tokenizer.
