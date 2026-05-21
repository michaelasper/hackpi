//! File watching, hot-reload triggers, and change detection for config files.
//!
//! This module is reserved for hot-reload logic that is conceptually
//! part of config management. The primary implementation (`HotReloader`,
//! `try_reload`, `validate`) currently lives in `crate::hot_reload` and
//! will be migrated here in a future change.
