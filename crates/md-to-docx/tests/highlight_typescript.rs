//! Integration checks for TypeScript fences in the generated Word package.
#[path = "common/mod.rs"]
mod common;

use md_to_docx::options::{CodeHighlightingOptions, OptionBool};
use md_to_docx::{convert_with_options, ConvertOptions};

#[test]
fn typescript_and_tsx_colors_reach_document_runs() {
    let options = ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(true)),
            ..Default::default()
        }),
        ..Default::default()
    };
    for (language, source, tokens) in [
        (
            "ts",
            "interface User { readonly name: string; }",
            vec![("interface", "6F42C1"), ("string", "6F42C1")],
        ),
        (
            "typescript",
            "type Count = number; const count: Count = 42;",
            vec![("number", "6F42C1"), ("42", "005CC5")],
        ),
        (
            "tsx",
            "const element = <div title=\"hello\">{42}</div>;",
            vec![("div", "22863A"), ("hello", "032F62")],
        ),
    ] {
        let bytes = convert_with_options(&format!("```{language}\n{source}\n```"), &options)
            .expect("conversion succeeds");
        let doc = common::project_docx(&bytes);
        let paragraphs = doc.paragraphs();
        let paragraph = paragraphs
            .iter()
            .find(|p| p.text.contains(source))
            .expect("source survives");
        for (token, expected) in tokens {
            assert!(
                paragraph
                    .runs
                    .iter()
                    .any(|run| run.text.contains(token) && run.color.as_deref() == Some(expected)),
                "{language}: expected {expected} on {token}, runs: {:?}",
                paragraph.runs
            );
        }
    }
}
