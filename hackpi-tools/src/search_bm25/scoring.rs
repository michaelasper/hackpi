/// BM25 scoring algorithm and IDF computation.
///
/// Provides the standard BM25 ranking formula parameters used during search.
///
/// Standard BM25 parameter: term frequency saturation factor.
pub(crate) const K1: f64 = 1.5;

/// Standard BM25 parameter: length normalization factor (0.0 = no normalization, 1.0 = full).
pub(crate) const B: f64 = 0.75;
