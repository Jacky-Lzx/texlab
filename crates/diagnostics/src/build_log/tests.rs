use rowan::TextRange;
use syntax::BuildErrorLevel;

use super::*;

fn document(text: &str) -> Document {
    let fixture = test_utils::fixture::Fixture::parse(&format!("%! main.tex\n{text}"));
    fixture
        .workspace
        .lookup(&Url::parse("file:///texlab/main.tex").unwrap())
        .unwrap()
        .clone()
}

fn undefined(hint: &str) -> BuildError {
    BuildError {
        relative_path: "./main.tex".into(),
        level: BuildErrorLevel::Error,
        message: "Undefined control sequence.".into(),
        hint: Some(hint.into()),
        line: Some(0),
    }
}

#[test]
fn test_undefined_command_range_excludes_context() {
    let doc = document("Before text \\bad after text.\n");
    assert_eq!(
        find_range_of_hint(&doc, &undefined("Before text \\bad")),
        Some(TextRange::new(12.into(), 16.into()))
    );
}

#[test]
fn test_truncated_unicode_context_uses_byte_offsets() {
    let doc = document("中文测试 Before \\bad after text.\n");
    let start = doc.text.find("\\bad").unwrap() as u32;
    assert_eq!(
        find_range_of_hint(&doc, &undefined("试 Before \\bad")),
        Some(TextRange::new(start.into(), (start + 4).into()))
    );
}

#[test]
fn test_tab_context() {
    let doc = document("\tBefore \\bad after text.\n");
    assert_eq!(
        find_range_of_hint(&doc, &undefined("Before \\bad")),
        Some(TextRange::new(8.into(), 12.into()))
    );
}

#[test]
fn test_repeated_truncated_context_is_not_guessed() {
    let doc = document("Before \\bad Before \\bad\n");
    assert_eq!(find_range_of_hint(&doc, &undefined("Before \\bad")), None);
}

#[test]
fn test_context_does_not_select_part_of_a_command() {
    let doc = document("\\badly\n");
    assert_eq!(find_range_of_hint(&doc, &undefined("\\bad")), None);
}

#[test]
fn test_macro_expansion_without_matching_source_is_not_guessed() {
    let doc = document("\\macro\n");
    assert_eq!(
        find_range_of_hint(&doc, &undefined("\\undefinedinside")),
        None
    );
}

#[test]
fn test_other_errors_keep_context_ranges() {
    let doc = document("Text x_1.\n");
    let mut error = undefined("Text x_");
    error.message = "Missing $ inserted.".into();
    assert_eq!(
        find_range_of_hint(&doc, &error),
        Some(TextRange::new(0.into(), 7.into()))
    );
}

fn workspace(input: &str) -> Workspace {
    let fixture = test_utils::fixture::Fixture::parse(input);
    // Fixture's virtual Unix URLs have no native path on Windows. Rebase the
    // documents onto a native absolute directory without touching the disk.
    let base =
        Url::from_directory_path(std::env::temp_dir().join("texlab-build-log-tests")).unwrap();
    let mut workspace = Workspace::default();
    for document in fixture.workspace.iter() {
        let path = document.uri.path().strip_prefix("/texlab/").unwrap();
        workspace.open(
            base.join(path).unwrap(),
            document.text.clone(),
            document.language,
            document.owner,
            document.cursor,
        );
    }
    workspace
}

fn diagnostics(input: &str) -> Vec<(String, Diagnostic)> {
    let workspace = workspace(input);
    let base =
        Url::from_directory_path(std::env::temp_dir().join("texlab-build-log-tests")).unwrap();
    let log = workspace
        .iter()
        .find(|d| d.data.as_log().is_some())
        .unwrap();
    let mut results = FxHashMap::default();
    assert_eq!(update(&workspace, log, &mut results), Some(()));
    results
        .into_values()
        .flat_map(|items| {
            items.into_iter().flat_map(|(uri, diagnostics)| {
                let uri = format!("file:///texlab/{}", base.make_relative(&uri).unwrap());
                diagnostics.into_iter().map(move |d| (uri.clone(), d))
            })
        })
        .collect()
}

#[test]
fn test_unresolved_file_is_reported_in_log_not_root_source() {
    let results = diagnostics(
        "%! main.tex\n\\documentclass{article}\n\\begin{document}\n%! main.log\n(./main.tex\n./missing.tex:8: Undefined control sequence.\nl.8 \\bad\n)",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "file:///texlab/main.log");
    assert!(
        results[0]
            .1
            .message()
            .contains("./missing.tex:8; source location unavailable")
    );
}

#[test]
fn test_out_of_range_line_is_not_replaced_by_first_source_line() {
    let results = diagnostics(
        "%! main.tex\n\\documentclass{article}\n\\begin{document}\n%! main.log\n(./main.tex\n! Undefined control sequence.\nl.99 \\bad\n)",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "file:///texlab/main.log");
    assert!(
        results[0]
            .1
            .message()
            .contains("./main.tex:99; source location unavailable")
    );
}

#[test]
fn test_parenthesized_child_file_keeps_its_own_location() {
    let results = diagnostics(
        "%! main.tex\n\\documentclass{article}\n\\begin{document}\n\\input{child(draft)}\n%! child(draft).tex\nBefore \\bad after\n%! main.log\n(./main.tex\n(./child(draft).tex\n! Undefined control sequence.\nl.1 Before \\bad\n)\n)",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "file:///texlab/child(draft).tex");
    assert!(
        matches!(&results[0].1, Diagnostic::Build(range, _) if *range == TextRange::new(7.into(), 11.into()))
    );
}

#[test]
fn test_changed_log_directory_discards_old_log_diagnostics() {
    let mut workspace = workspace(
        "%! main.tex\n\\begin{document}\n\\bad\n\\end{document}\n%! main.log\n(./main.tex\n! Undefined control sequence.\nl.2 \\bad\n)\n%! build/main.log\nA successful build.\n",
    );
    let mut manager = crate::Manager::default();
    for document in workspace.iter() {
        manager.update_syntax(&workspace, document);
    }
    assert!(
        manager
            .get(&workspace)
            .values()
            .flatten()
            .any(|d| matches!(d, Diagnostic::Build(_, _)))
    );
    let mut config = base_db::Config::default();
    config.build.log_dir = "missing-build".into();
    workspace.set_config(config);
    // Preserve fallback discovery when the configured log does not exist yet.
    assert!(
        manager
            .get(&workspace)
            .values()
            .flatten()
            .any(|d| matches!(d, Diagnostic::Build(_, _)))
    );
    let mut config = base_db::Config::default();
    config.build.log_dir = "build".into();
    workspace.set_config(config);
    assert!(
        !manager
            .get(&workspace)
            .values()
            .flatten()
            .any(|d| matches!(d, Diagnostic::Build(_, _)))
    );
}

#[test]
fn test_source_edit_does_not_reuse_stale_byte_ranges() {
    let mut workspace = workspace(
        "%! main.tex\n\\begin{document}\nBefore \\undefinedtestcommand after\n\\end{document}\n%! main.log\n(./main.tex\n! Undefined control sequence.\nl.2 Before \\undefinedtestcommand\n)\n",
    );
    let mut manager = crate::Manager::default();
    for document in workspace.iter() {
        manager.update_syntax(&workspace, document);
    }
    let source = workspace
        .iter()
        .find(|d| d.data.as_tex().is_some())
        .unwrap()
        .clone();
    let updated = "\\begin{document}\nok\n\\end{document}\n";
    workspace.open(
        source.uri.clone(),
        updated.into(),
        source.language,
        source.owner,
        source.cursor,
    );
    let start = TextSize::try_from(updated.find("ok").unwrap()).unwrap();
    let results = manager.get(&workspace);
    let ranges: Vec<_> = results[&source.uri]
        .iter()
        .filter_map(|d| match d {
            Diagnostic::Build(range, _) => Some(*range),
            _ => None,
        })
        .collect();
    assert_eq!(ranges, vec![TextRange::empty(start)]);
}
