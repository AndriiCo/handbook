//! Thin watch ingest (F1/F2): fetch bytes, dedup by sha256, store a
//! `warc` capture + text sidecar + FTS row.
//!
//! Day-2 scope only: single-URL fetch, plain-blob WARC (raw bytes under
//! their sha256 address, no WARC envelope yet), naive tag-stripped text
//! extraction, and a minimal robots.txt gate for third-party hosts.
//! Full 500-URL crawl and politeness scheduling are Week-3 work.

pub mod capture;
pub mod fetch;
pub mod hcch;
pub mod seed;
pub mod simhash;
pub mod sweep;
pub mod warc;
