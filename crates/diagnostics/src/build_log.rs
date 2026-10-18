use base_db::{Document, Workspace, deps};
use line_index::LineCol;
use multimap::MultiMap;
use rowan::{TextLen, TextRange, TextSize};
use rustc_hash::FxHashMap;
use syntax::{BuildError, latex};
use url::Url;

use crate::types::Diagnostic;

pub fn update(
    workspace: &Workspace,
    log_document: &Document,
    results: &mut FxHashMap<Url, MultiMap<Url, Diagnostic>>,
) -> Option<()> {
    let mut errors = MultiMap::default();

    let data = log_document.data.as_log()?;

    let parents = deps::parents(workspace, log_document);
    let root_document = parents.iter().next()?;

    let base_path = root_document
        .path
        .as_deref()
        .and_then(|path| path.parent())?;

    for error in &data.errors {
        let full_path = base_path.join(&error.relative_path);
        let Ok(full_path_uri) = Url::from_file_path(&full_path) else {
            continue;
        };

        let location = workspace.lookup(&full_path_uri).and_then(|document| {
            let range = find_range_of_hint(document, error).or_else(|| {
                let line = error.line?;
                document
                    .line_index
                    .offset(LineCol { line, col: 0 })
                    .map(TextRange::empty)
            })?;
            Some((document, range))
        });

        let (document, range, error) = if let Some((document, range)) = location {
            (document, range, error.clone())
        } else {
            // Keep unresolved diagnostics visible in the log. Assigning an
            // unknown file or an out-of-date line to the root file invents a
            // source location (often line 1) that the compiler never reported.
            let start = log_document.text.find(&error.message).unwrap_or(0);
            let start = TextSize::try_from(start).unwrap();
            let line = error
                .line
                .map(|line| format!(":{}", u64::from(line) + 1))
                .unwrap_or_default();
            let mut error = error.clone();
            error.message = format!(
                "{} (reported in {}{}; source location unavailable)",
                error.message,
                error.relative_path.display(),
                line
            );
            (log_document, TextRange::empty(start), error)
        };
        errors.insert(document.uri.clone(), Diagnostic::Build(range, error));
    }

    results.insert(log_document.uri.clone(), errors);
    Some(())
}

fn find_range_of_hint(document: &Document, error: &BuildError) -> Option<TextRange> {
    let line = error.line?;
    let hint = error.hint.as_deref()?;
    let line_index = &document.line_index;

    let line_start = line_index.offset(LineCol { line, col: 0 })?;
    let line_end = line_index
        .offset(LineCol {
            line: line + 1,
            col: 0,
        })
        .unwrap_or_else(|| document.text.text_len());

    let line_text = &document.text[line_start.into()..line_end.into()];
    let start = line_text.find(hint)?;
    if hint.is_empty() || line_text.rfind(hint)? != start {
        return None; // Truncated or repeated source context is ambiguous.
    }
    let hint_start = line_start + TextSize::try_from(start).ok()?;
    let hint_end = hint_start + hint.text_len();
    if error.message == "Undefined control sequence." {
        let data = document.data.as_tex()?;
        let root = latex::SyntaxNode::new_root(data.green.clone());
        let token = root.token_at_offset(hint_end).left_biased()?;
        return (token.kind() == latex::COMMAND_NAME && token.text_range().end() == hint_end)
            .then(|| token.text_range());
    }
    Some(TextRange::new(hint_start, hint_end))
}

#[cfg(test)]
mod tests;
