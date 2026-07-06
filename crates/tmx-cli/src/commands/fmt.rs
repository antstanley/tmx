//! `tmx fmt` — reformat / convert an artifact behind [`FormatArtifact`] (07 §`tmx fmt`).
//!
//! Loads the artifact through the [`SourceLoader`] into the one shared model and re-emits it: to a
//! `--to` target format (loss-free across YAML/JSON/JSONC/TOML — a re-parse of the output lands in
//! the identical model), or in its own format when `--to` is absent (a pure reformat). By default
//! the formatted text is printed to stdout; `--write` writes it back to the source file (swapping the
//! extension on a `--to` conversion). This is TMX's defining property made a command: four formats,
//! one model, converted without loss.

use std::path::Path;

use async_trait::async_trait;

use tmx_adapters::loader::{FileSourceLoader, detect_source_kind, emit_source};

use tmx_core::RunError;
use tmx_core::ports::driven::{SourceKind, SourceLoader};
use tmx_core::ports::driving::FormatArtifact;

use crate::args::{FmtArgs, RunArgs};
use crate::commands::run::resolve_target;
use crate::config;

/// The `FormatArtifact` use case over the built-in loader + emitter.
pub struct EngineFormatArtifact {
    loader: FileSourceLoader,
}

impl EngineFormatArtifact {
    /// A fresh use case.
    #[must_use]
    pub fn new() -> Self {
        Self {
            loader: FileSourceLoader::new(),
        }
    }
}

impl Default for EngineFormatArtifact {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FormatArtifact for EngineFormatArtifact {
    async fn format(&self, path: &str, to: Option<SourceKind>) -> Result<String, RunError> {
        let source_kind = detect_source_kind(path)?;
        let value = self.loader.load(path, source_kind).await?;
        // `--to` converts; its absence reformats in the source's own format.
        emit_source(&value, to.unwrap_or(source_kind))
    }
}

/// The outcome of a `tmx fmt`: the formatted text, and the path it was written to (when `--write`).
pub struct FmtOutput {
    /// The formatted / converted artifact text.
    pub text: String,
    /// The file the text was written to, when `--write` was set; `None` means it goes to stdout.
    pub written_to: Option<String>,
}

/// Run `tmx fmt`, returning the formatted text (and, under `--write`, the file it was written to).
///
/// # Errors
///
/// Returns `resolution` for an unresolved / unreadable artifact, or `validation` for one that does
/// not parse or cannot be represented in the target format.
pub async fn execute(args: FmtArgs) -> Result<FmtOutput, RunError> {
    let path = resolve_path(&args)?;
    let to = args.to.map(crate::args::SourceKindArg::to_kind);
    let use_case = EngineFormatArtifact::new();
    let text = use_case.format(&path, to).await?;

    if !args.write {
        return Ok(FmtOutput {
            text,
            written_to: None,
        });
    }

    // `--write`: overwrite in place, swapping the extension when `--to` converted the format.
    let target = match to {
        Some(kind) => swap_extension(&path, kind),
        None => path.clone(),
    };
    std::fs::write(&target, &text).map_err(|e| {
        RunError::resolution(
            "fmt_unwritable",
            format!("could not write formatted artifact `{target}`: {e}"),
        )
    })?;
    Ok(FmtOutput {
        text,
        written_to: Some(target),
    })
}

/// Resolve the artifact path: the positional `path`, else the Flow resolved by the `tmx run` search
/// order (a single file — a directory layout has no single artifact to format).
fn resolve_path(args: &FmtArgs) -> Result<String, RunError> {
    if let Some(path) = &args.path {
        return Ok(path.clone());
    }
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let resolved = resolve_target(&RunArgs::default(), &cwd, config::env_flow())?;
    resolved.file_reference.ok_or_else(|| {
        RunError::resolution(
            "fmt_requires_file",
            "tmx fmt formats a single artifact file, not a directory layout",
        )
    })
}

/// Rewrite `path`'s extension to the one `kind` names — the file `--write --to` produces.
fn swap_extension(path: &str, kind: SourceKind) -> String {
    let ext = extension_for(kind);
    let stem = Path::new(path)
        .with_extension("")
        .to_string_lossy()
        .into_owned();
    format!("{stem}.{ext}")
}

/// The canonical file extension for a wire format.
fn extension_for(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Yaml => "yaml",
        SourceKind::Json => "json",
        SourceKind::Jsonc => "jsonc",
        SourceKind::Toml => "toml",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_adapters::loader::parse_source;

    /// A minimal async block-on for the `format` use case (immediately-ready futures — the loader's
    /// `std::fs` read is synchronous).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll};
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            if let Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
                return value;
            }
        }
    }

    #[test]
    fn fmt_round_trips_a_flow_across_all_four_formats_without_loss() {
        let dir = std::env::temp_dir().join(format!("tmx-fmt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        // A YAML source whose top-level keys list scalars before tables, so every target (TOML too)
        // can represent it.
        let source = dir.join("flow.yaml");
        std::fs::write(
            &source,
            "name: demo\nversion: \"1\"\ntasks:\n  - name: build\n    type: exec\n    with:\n      command: echo hi\n",
        )
        .expect("write source");
        let source_str = source.to_str().expect("utf8 path").to_string();

        let original = {
            let loader = FileSourceLoader::new();
            block_on(loader.load(&source_str, SourceKind::Yaml)).expect("load source")
        };
        let use_case = EngineFormatArtifact::new();
        for kind in [
            SourceKind::Yaml,
            SourceKind::Json,
            SourceKind::Jsonc,
            SourceKind::Toml,
        ] {
            let text = block_on(use_case.format(&source_str, Some(kind))).expect("fmt converts");
            let reloaded = parse_source(&text, kind).expect("the converted text re-parses");
            assert_eq!(
                reloaded, original,
                "a fmt conversion to {kind:?} preserves the model exactly"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn swap_extension_rewrites_to_the_target_format() {
        assert_eq!(
            swap_extension("dir/flow.yaml", SourceKind::Toml),
            "dir/flow.toml",
            "the extension is swapped to the target format"
        );
        assert_eq!(
            swap_extension("flow.json", SourceKind::Yaml),
            "flow.yaml",
            "a bare filename's extension is swapped"
        );
    }
}
