#![forbid(unsafe_code)]
//! `tmx-schema` — the TMX input data model.
//!
//! Deserialise-only types mirroring the frozen `tmx.schema.json` data-model contract — `Flow`,
//! `Task`, `TaskWith`, `Context`, `Environment`, and the `MatcherName` vocabulary — plus the
//! single source of truth for every runtime **limit** constant (`STATE_SIZE_MAX_BYTES`,
//! `FLOW_DEPTH_MAX`, …). Pure data: no I/O, no async, and no dependency pointing outward, so it
//! sits at the bottom of the workspace dependency graph.
//!
//! Ports: none. This crate declares no port; it is the shared vocabulary that `tmx-core` and the
//! adapters both speak. The types and limits themselves arrive in the schema tasks (02–03).
