//! Slice 9 parity for plugin registration rules.
//!
//! Plugins are trusted Rust values passed on [`ConvertOptions`]. Output checks
//! use the paragraph projection. ZIP bytes are not compared.
//!
//! CLI tests are not in this file. The workspace has no `md-to-docx-cli` crate,
//! so the Jest case "fails when options file contains plugin configuration"
//! (`{"plugins":[]}` must fail with "cannot be loaded from CLI JSON") is not
//! sent. JavaScript plugin objects are a later host.

#[path = "common/mod.rs"]
mod common;

use std::sync::{Arc, Mutex};

use common::Document;

use md_to_docx::options::{
    ChartRenderingOptions, ConvertOptions, MarkdownDocxPlugin, MermaidRenderingOptions, OptionBool,
    PluginAstTransform, PluginBlock, PluginBlockNodeHandler, PluginConflictPolicy,
    PluginFailureMode, PluginFenceHandler, PluginFenceInput, PluginInline, PluginOptions,
};
use md_to_docx::{convert_with_options, MarkdownConversionError};

fn project(bytes: &[u8]) -> Document {
    assert!(
        bytes.starts_with(b"PK"),
        "conversion should return a zip package"
    );
    common::project_docx(bytes)
}

fn convert(markdown: &str, options: &ConvertOptions) -> Document {
    project(
        &convert_with_options(markdown, options)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn convert_err(markdown: &str, options: &ConvertOptions) -> MarkdownConversionError {
    convert_with_options(markdown, options).expect_err("conversion should fail")
}

fn paragraph(text: impl Into<String>, bold: bool) -> PluginBlock {
    PluginBlock::Paragraph {
        children: vec![PluginInline {
            text: text.into(),
            bold,
            ..PluginInline::default()
        }],
    }
}

fn fence_handler(
    languages: &[&str],
    failure_mode: Option<PluginFailureMode>,
    render: impl Fn(&PluginFenceInput) -> Result<Vec<PluginBlock>, MarkdownConversionError>
        + Send
        + Sync
        + 'static,
) -> PluginFenceHandler {
    PluginFenceHandler {
        languages: languages
            .iter()
            .map(|language| (*language).to_string())
            .collect(),
        failure_mode,
        render: PluginFenceHandler::render_fn(render),
    }
}

fn fence_plugin(
    name: &str,
    languages: &[&str],
    priority: Option<i32>,
    failure_mode: Option<PluginFailureMode>,
    render: impl Fn(&PluginFenceInput) -> Result<Vec<PluginBlock>, MarkdownConversionError>
        + Send
        + Sync
        + 'static,
) -> MarkdownDocxPlugin {
    MarkdownDocxPlugin {
        api_version: 1,
        name: name.to_string(),
        priority,
        setup: None,
        transform_ast: None,
        fenced_blocks: Some(vec![fence_handler(languages, failure_mode, render)]),
        block_nodes: None,
    }
}

fn static_fence(
    name: &str,
    language: &str,
    priority: Option<i32>,
    text: &'static str,
) -> MarkdownDocxPlugin {
    fence_plugin(name, &[language], priority, None, move |_input| {
        Ok(vec![paragraph(text, false)])
    })
}

fn block_plugin(name: &str, node_types: &[&str], priority: Option<i32>) -> MarkdownDocxPlugin {
    MarkdownDocxPlugin {
        api_version: 1,
        name: name.to_string(),
        priority,
        setup: None,
        transform_ast: None,
        fenced_blocks: None,
        block_nodes: Some(vec![PluginBlockNodeHandler {
            node_types: node_types
                .iter()
                .map(|node_type| (*node_type).to_string())
                .collect(),
            failure_mode: None,
            render: PluginBlockNodeHandler::render_fn(|_input| Ok(vec![PluginBlock::Skip])),
        }]),
    }
}

fn with_plugins(
    plugins: Vec<MarkdownDocxPlugin>,
    policy: Option<PluginConflictPolicy>,
) -> ConvertOptions {
    ConvertOptions {
        plugins: Some(plugins),
        plugin_options: policy.map(|conflict_policy| PluginOptions {
            conflict_policy: Some(conflict_policy),
        }),
        ..ConvertOptions::default()
    }
}

fn texts(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .map(|paragraph| paragraph.text.clone())
        .filter(|text| !text.is_empty())
        .collect()
}

fn contains_text(doc: &Document, needle: &str) -> bool {
    texts(doc).iter().any(|text| text.contains(needle))
}

// Jest: "renders a custom fenced block with semantic content"
// tests/plugin-api.test.ts
#[test]
fn renders_a_custom_fenced_block() {
    let doc = convert(
        "```notice\nShip it\n```",
        &with_plugins(
            vec![fence_plugin(
                "acme.notice",
                &["notice"],
                None,
                None,
                |input| Ok(vec![paragraph(format!("NOTICE: {}", input.value), true)]),
            )],
            None,
        ),
    );
    assert!(contains_text(&doc, "NOTICE: Ship it"));
    assert!(
        doc.paragraphs().into_iter().any(|paragraph| {
            paragraph
                .runs
                .iter()
                .any(|run| run.text.contains("NOTICE: Ship it") && run.bold)
        }),
        "plugin paragraph text should be bold"
    );
    assert!(!contains_text(&doc, "notice"));
}

// Jest: "uses priority then registration order only when conflicts are explicitly allowed"
// tests/plugin-api.test.ts
#[test]
fn priority_then_registration_order_when_conflicts_are_allowed() {
    let low = static_fence("acme.low", "notice", Some(1), "low");
    let high = static_fence("acme.high", "notice", Some(10), "high");
    let conflict = convert_err("```notice\nx\n```", &with_plugins(vec![low, high], None));
    assert!(conflict
        .to_string()
        .contains("Conflicting plugin fence handlers"));

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut low = static_fence("acme.low", "notice", Some(1), "low");
    let mut high = static_fence("acme.high", "notice", Some(10), "high");
    let low_order = Arc::clone(&order);
    let high_order = Arc::clone(&order);
    low.transform_ast = Some(PluginAstTransform::new(move |_| {
        low_order.lock().expect("order").push("low");
    }));
    high.transform_ast = Some(PluginAstTransform::new(move |_| {
        high_order.lock().expect("order").push("high");
    }));
    let doc = convert(
        "```notice\nx\n```",
        &with_plugins(vec![low, high], Some(PluginConflictPolicy::UsePriority)),
    );
    assert!(contains_text(&doc, "high"));
    assert!(!contains_text(&doc, "low"));
    assert_eq!(order.lock().expect("order").as_slice(), ["high", "low"]);

    let first = static_fence("acme.first", "notice", None, "first");
    let second = static_fence("acme.second", "notice", None, "second");
    let equal = convert(
        "```notice\nx\n```",
        &with_plugins(vec![first, second], Some(PluginConflictPolicy::UsePriority)),
    );
    assert!(contains_text(&equal, "first"));
    assert!(!contains_text(&equal, "second"));
}

// Jest: "rejects duplicate plugin names, duplicate handlers, and built-in collisions"
// tests/plugin-api.test.ts
#[test]
fn rejects_duplicate_names_handlers_and_builtin_collisions() {
    let duplicate_name = convert_err(
        "text",
        &with_plugins(
            vec![
                static_fence("acme.same", "notice", None, "one"),
                static_fence("acme.same", "notice", None, "two"),
            ],
            None,
        ),
    );
    assert!(duplicate_name.to_string().contains("Duplicate plugin name"));

    let duplicate_handler = convert_err(
        "text",
        &with_plugins(
            vec![fence_plugin(
                "acme.duplicate-handler",
                &["same", "SAME"],
                None,
                None,
                |_input| Ok(vec![PluginBlock::Skip]),
            )],
            None,
        ),
    );
    assert!(duplicate_handler
        .to_string()
        .contains("Duplicate plugin fence handler name"));

    let across = MarkdownDocxPlugin {
        api_version: 1,
        name: "acme.duplicate-across-handlers".to_string(),
        priority: None,
        setup: None,
        transform_ast: None,
        fenced_blocks: Some(vec![
            fence_handler(&["same"], None, |_input| Ok(vec![PluginBlock::Skip])),
            fence_handler(&["SAME"], None, |_input| Ok(vec![PluginBlock::Skip])),
        ]),
        block_nodes: None,
    };
    let duplicate_across = convert_err(
        "text",
        &with_plugins(vec![across], Some(PluginConflictPolicy::UsePriority)),
    );
    assert!(duplicate_across
        .to_string()
        .contains("Duplicate plugin fence handler"));
    assert!(!duplicate_across
        .to_string()
        .contains("Duplicate plugin fence handler name"));

    let mermaid = convert_err(
        "```mermaid\ngraph TD\n```",
        &ConvertOptions {
            mermaid_rendering: Some(MermaidRenderingOptions {
                enabled: Some(OptionBool::Bool(true)),
                ..MermaidRenderingOptions::default()
            }),
            ..with_plugins(
                vec![static_fence("acme.mermaid", "mermaid", None, "skip")],
                None,
            )
        },
    );
    assert!(mermaid
        .to_string()
        .contains("conflicts with an enabled built-in renderer"));

    // Plan: enabled `chart` and `chartjs` fences cannot be claimed. The Jest
    // case above covers mermaid only.
    for language in ["chart", "chartjs"] {
        let chart = convert_err(
            "text",
            &ConvertOptions {
                chart_rendering: Some(ChartRenderingOptions {
                    enabled: Some(OptionBool::Bool(true)),
                    ..ChartRenderingOptions::default()
                }),
                ..with_plugins(
                    vec![static_fence(
                        &format!("acme.{language}"),
                        language,
                        None,
                        "skip",
                    )],
                    None,
                )
            },
        );
        assert!(
            chart
                .to_string()
                .contains("conflicts with an enabled built-in renderer"),
            "{language}: {chart}"
        );
    }

    for node_type in ["definition", "yaml"] {
        let built_in = convert_err(
            "text",
            &with_plugins(
                vec![block_plugin(
                    &format!("acme.{node_type}"),
                    &[node_type],
                    None,
                )],
                None,
            ),
        );
        assert!(
            built_in
                .to_string()
                .contains("cannot override a built-in node type"),
            "{node_type}: {built_in}"
        );
    }
}

// Jest: "applies the %s failure policy"
// tests/plugin-api.test.ts
#[test]
fn applies_fallback_skip_and_throw_failure_policies() {
    let exploding = |mode: PluginFailureMode| {
        fence_plugin("acme.failure", &["notice"], None, Some(mode), |_input| {
            Err(MarkdownConversionError::new("renderer exploded"))
        })
    };

    let fallback = convert(
        "```notice\nsource\n```",
        &with_plugins(vec![exploding(PluginFailureMode::Fallback)], None),
    );
    assert!(contains_text(&fallback, "source"));

    let skip = convert(
        "```notice\nsource\n```",
        &with_plugins(vec![exploding(PluginFailureMode::Skip)], None),
    );
    assert!(!contains_text(&skip, "source"));

    let thrown = convert_err(
        "```notice\nsource\n```",
        &with_plugins(vec![exploding(PluginFailureMode::Throw)], None),
    );
    assert!(thrown
        .to_string()
        .contains("Plugin \"acme.failure\" fence renderer failed: renderer exploded"));
}

// Plan: priority is an integer from -1000 to 1000.
#[test]
fn rejects_plugin_priority_outside_the_allowed_range() {
    for priority in [1000, -1000] {
        let doc = convert(
            "```notice\nx\n```",
            &with_plugins(
                vec![static_fence("acme.edge", "notice", Some(priority), "edge")],
                None,
            ),
        );
        assert!(contains_text(&doc, "edge"), "priority {priority}");
    }
    for priority in [1001, -1001] {
        let err = convert_err(
            "text",
            &with_plugins(
                vec![static_fence("acme.edge", "notice", Some(priority), "edge")],
                None,
            ),
        );
        assert!(
            err.to_string()
                .contains("Invalid plugin priority: must be an integer from -1000 to 1000"),
            "{priority}: {err}"
        );
    }
}

// Plan: plugin names are unique lowercase identifiers. API version is 1.
#[test]
fn rejects_invalid_plugin_names_and_api_versions() {
    let invalid_name = convert_err(
        "text",
        &with_plugins(vec![static_fence("Acme", "notice", None, "nope")], None),
    );
    assert!(invalid_name
        .to_string()
        .contains("Invalid plugin name: use a stable lowercase identifier"));

    let mut plugin = static_fence("acme.notice", "notice", None, "nope");
    plugin.api_version = 2;
    let bad_version = convert_err("text", &with_plugins(vec![plugin], None));
    assert!(bad_version
        .to_string()
        .contains("Unsupported plugin apiVersion"));
}

// Plan: conflictPolicy is `error` or `use-priority`. Block-node handlers follow
// the same rule as fences.
#[test]
fn block_node_conflicts_follow_the_conflict_policy() {
    let conflict = convert_err(
        "text",
        &with_plugins(
            vec![
                block_plugin("acme.low", &["noteDirective"], Some(1)),
                block_plugin("acme.high", &["noteDirective"], Some(10)),
            ],
            None,
        ),
    );
    assert!(conflict
        .to_string()
        .contains("Conflicting plugin block-node handlers"));

    let doc = convert(
        "text",
        &with_plugins(
            vec![
                block_plugin("acme.low", &["noteDirective"], Some(1)),
                block_plugin("acme.high", &["noteDirective"], Some(10)),
            ],
            Some(PluginConflictPolicy::UsePriority),
        ),
    );
    assert!(contains_text(&doc, "text"));
}

// Jest: "preserves no-plugin output and supports reference-DOCX patching"
// tests/plugin-api.test.ts
//
// The patch half needs a plugin on the patch options. This covers the empty
// plugin list, which must match a conversion that omits plugins.
#[test]
fn empty_plugin_list_matches_a_conversion_without_plugins() {
    let markdown = "# Stable\n\n```mermaid\ngraph TD\n```";
    let without = convert(markdown, &ConvertOptions::default());
    let empty = convert(
        markdown,
        &ConvertOptions {
            plugins: Some(vec![]),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(texts(&without), texts(&empty));
    assert!(contains_text(&without, "graph TD"));
}
