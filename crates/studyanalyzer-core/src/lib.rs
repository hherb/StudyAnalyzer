//! StudyAnalyzer's core: plain functions over bmlib, testable without Tauri.

pub mod answer;
pub mod bookmark;
pub mod config;
pub mod conversations;
pub mod embedder;
pub mod handoff;
pub mod handoff_store;
pub mod identifiers;
pub mod intake;
pub mod interrogate;
pub mod library;
pub mod library_setting;
pub mod limit;
pub mod llm_settings;
pub mod open;
pub mod passages;
pub mod pico;
pub mod pico_store;
pub mod prompt;
pub mod retrieval;
pub mod retry;
pub mod schema;
pub mod store;
pub mod view;
