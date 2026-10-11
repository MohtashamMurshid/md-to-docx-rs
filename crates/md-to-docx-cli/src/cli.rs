//! CLI runtime ported from `src/cli.ts`.
//!
//! Status text goes to stderr when stdout is the docx bytes. Batch keeps
//! going after a file fails, then exits 1. Writes land in a temp file and
//! are renamed into place. Watch waits 100 ms after the last relevant change.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use md_to_docx::{
    convert_with_options, convert_with_reference, patch_markdown, ReferenceDocxGenerationOptions,
};
use notify::Watcher;

use crate::args::{parse_command, resolve_path, Command, ParsedArgs, HELP_TEXT};
use crate::json_options::{load_options_json, load_patches_json, LoadedOptions};
use crate::locale_sort::js_locale_compare;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

type StdoutWriter = Box<dyn FnMut(&[u8]) -> Result<(), String>>;

pub struct CliHooks {
    pub log: Box<dyn FnMut(&str)>,
    pub error: Box<dyn FnMut(&str)>,
    pub read_stdin: Option<Box<dyn FnMut() -> Result<String, String>>>,
    pub write_stdout: Option<StdoutWriter>,
}

impl CliHooks {
    pub fn stdio() -> Self {
        Self {
            log: Box::new(|message| println!("{message}")),
            error: Box::new(|message| eprintln!("{message}")),
            read_stdin: None,
            write_stdout: None,
        }
    }
}

pub fn run_cli(args: &[String], hooks: &mut CliHooks, abort: &AtomicBool) -> i32 {
    match parse_command(args) {
        Ok(Command::Help) => {
            (hooks.log)(HELP_TEXT);
            0
        }
        Ok(Command::Run(parsed)) => match execute(&parsed, hooks, abort) {
            Ok(()) => 0,
            Err(message) => fail(hooks, &message),
        },
        Err(message) => fail(hooks, &message),
    }
}

fn fail(hooks: &mut CliHooks, message: &str) -> i32 {
    (hooks.error)(&format!("Error: {message}"));
    (hooks.error)(HELP_TEXT);
    1
}

fn execute(parsed: &ParsedArgs, hooks: &mut CliHooks, abort: &AtomicBool) -> Result<(), String> {
    if let Err(message) = convert_all(parsed, hooks, abort) {
        if !parsed.watch {
            return Err(message);
        }
        (hooks.error)(&format!("Error: {message}"));
    }
    if !parsed.watch || abort.load(Ordering::SeqCst) {
        return Ok(());
    }
    watch(parsed, hooks, abort)
}

fn convert_all(
    parsed: &ParsedArgs,
    hooks: &mut CliHooks,
    abort: &AtomicBool,
) -> Result<(), String> {
    let loaded = match &parsed.options {
        Some(path) => Some(read_options(path)?),
        None => None,
    };
    let inputs = if parsed.batch {
        let files = collect_markdown(&resolve_path(&parsed.input))?;
        if files.is_empty() {
            return Err("No Markdown files found".to_string());
        }
        files
    } else {
        vec![parsed.input.clone()]
    };
    let reference = match &parsed.reference {
        Some(path) => Some(fs::read(path).map_err(|error| io_message(Path::new(path), &error))?),
        None => None,
    };
    let binary_stdout = parsed.output == "-";
    let mut failed = false;
    for input in &inputs {
        if abort.load(Ordering::SeqCst) {
            return Ok(());
        }
        match convert_one(
            parsed,
            loaded.as_ref(),
            reference.as_deref(),
            input,
            hooks,
            abort,
        ) {
            Ok(bytes) => {
                if binary_stdout {
                    write_binary_stdout(hooks, &bytes)?;
                } else {
                    let destination = destination_path(parsed, input);
                    atomic_write(&destination, &bytes)?;
                    report(
                        hooks,
                        binary_stdout,
                        &format!("DOCX created at: {}", destination.display()),
                    );
                }
            }
            Err(message) => {
                if !parsed.batch {
                    return Err(message);
                }
                failed = true;
                (hooks.error)(&format!("{input}: {message}"));
            }
        }
    }
    if failed {
        return Err("One or more batch conversions failed".to_string());
    }
    Ok(())
}

fn convert_one(
    parsed: &ParsedArgs,
    loaded: Option<&LoadedOptions>,
    reference: Option<&[u8]>,
    input: &str,
    hooks: &mut CliHooks,
    abort: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut convert_options = loaded
        .map(|loaded| loaded.convert.clone())
        .unwrap_or_default();
    let mut patch_options = loaded
        .map(|loaded| loaded.patch.clone())
        .unwrap_or_default();
    let base_directory = image_base_directory(parsed, loaded, input);
    let images = convert_options
        .image_handling
        .get_or_insert_with(Default::default);
    images.base_directory = Some(base_directory);
    patch_options.convert = convert_options.clone();
    if let Some(patches) = &parsed.patches {
        let template = fs::read(input).map_err(|error| io_message(Path::new(input), &error))?;
        let text =
            fs::read_to_string(patches).map_err(|error| io_message(Path::new(patches), &error))?;
        let patches = load_patches_json(&text)?;
        return patch_markdown(&template, &patches, &patch_options).map_err(error_message);
    }
    let markdown = if input == "-" {
        read_standard_input(hooks)?
    } else {
        fs::read_to_string(input).map_err(|error| io_message(Path::new(input), &error))?
    };
    if let Some(reference) = reference {
        let options = ReferenceDocxGenerationOptions {
            convert: convert_options,
            reference: loaded.and_then(|loaded| loaded.reference.clone()),
            aborted: abort.load(Ordering::SeqCst),
        };
        return convert_with_reference(&markdown, reference, &options).map_err(error_message);
    }
    let _ = patch_options;
    convert_with_options(&markdown, &convert_options).map_err(error_message)
}

/// Resolve the image root relative to the input file.
///
/// An explicit CLI path takes precedence over JSON and the input directory.
fn image_base_directory(
    parsed: &ParsedArgs,
    loaded: Option<&LoadedOptions>,
    input: &str,
) -> PathBuf {
    if let Some(base) = &parsed.base_directory {
        return resolve_path(base);
    }
    if let Some(base) = loaded.and_then(|loaded| loaded.image_base_directory.as_deref()) {
        return resolve_path(base);
    }
    if input == "-" {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }
    resolve_path(input)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn read_options(path: &str) -> Result<LoadedOptions, String> {
    let text = fs::read_to_string(path).map_err(|error| io_message(Path::new(path), &error))?;
    load_options_json(&text, path)
}

fn destination_path(parsed: &ParsedArgs, input: &str) -> PathBuf {
    if parsed.batch {
        let root = resolve_path(&parsed.input);
        let file = resolve_path(input);
        let relative = file.strip_prefix(&root).unwrap_or(file.as_path());
        resolve_path(&parsed.output).join(replace_md_suffix(relative))
    } else {
        resolve_path(&parsed.output)
    }
}

fn ends_with_md(name: &str) -> bool {
    let suffix = b".md";
    name.len() >= suffix.len()
        && name.as_bytes()[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn replace_md_suffix(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    if ends_with_md(name) {
        let mut output = path.to_path_buf();
        output.set_file_name(format!("{}.docx", &name[..name.len() - 3]));
        return output;
    }
    path.to_path_buf()
}

fn collect_markdown(directory: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_files(directory, &mut files)?;
    Ok(files
        .into_iter()
        .filter(|file| {
            Path::new(file)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(ends_with_md)
        })
        .collect())
}

fn collect_files(directory: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| io_message(directory, &error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_message(directory, &error))?;
    entries.sort_by(|left, right| {
        js_locale_compare(
            &left.file_name().to_string_lossy(),
            &right.file_name().to_string_lossy(),
        )
    });
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| io_message(&path, &error))?;
        if file_type.is_dir() {
            collect_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

fn atomic_write(file: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| io_message(parent, &error))?;
        }
    }
    static TOKEN: AtomicU64 = AtomicU64::new(1);
    let token = TOKEN.fetch_add(1, Ordering::Relaxed);
    let mut temporary = file.as_os_str().to_os_string();
    temporary.push(format!(".{}.{token}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let result = (|| -> Result<(), String> {
        fs::write(&temporary, data).map_err(|error| io_message(&temporary, &error))?;
        fs::rename(&temporary, file).map_err(|error| io_message(file, &error))?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn read_standard_input(hooks: &mut CliHooks) -> Result<String, String> {
    if let Some(read) = hooks.read_stdin.as_mut() {
        return read();
    }
    let mut buffer = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buffer)
        .map_err(|error| error.to_string())?;
    String::from_utf8(buffer).map_err(|_| "stdin is not valid UTF-8".to_string())
}

fn write_binary_stdout(hooks: &mut CliHooks, data: &[u8]) -> Result<(), String> {
    if let Some(write) = hooks.write_stdout.as_mut() {
        return write(data);
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(data).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

fn report(hooks: &mut CliHooks, binary_stdout: bool, message: &str) {
    if binary_stdout {
        (hooks.error)(message);
    } else {
        (hooks.log)(message);
    }
}

fn watch(parsed: &ParsedArgs, hooks: &mut CliHooks, abort: &AtomicBool) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
            let _ = sender.send(event);
        })
        .map_err(|error| error.to_string())?;
    let mut watched = std::collections::BTreeSet::new();
    sync_watch_roots(&mut watcher, &mut watched, parsed)?;
    report(hooks, parsed.output == "-", "Watching for changes...");
    let mut deadline: Option<Instant> = None;
    while !abort.load(Ordering::SeqCst) {
        let timeout = match deadline {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            None => WATCH_DEBOUNCE,
        };
        if deadline.is_some() && timeout.is_zero() {
            deadline = None;
            rebuild(parsed, hooks, abort, &mut watcher, &mut watched);
            continue;
        }
        match receiver.recv_timeout(timeout.max(Duration::from_millis(1))) {
            Ok(Ok(event)) => {
                if relevant_event(&event, parsed) {
                    deadline = Some(Instant::now() + WATCH_DEBOUNCE);
                }
            }
            Ok(Err(error)) => (hooks.error)(&format!("Watch error: {error}")),
            Err(RecvTimeoutError::Timeout) => {
                if deadline.is_some() {
                    deadline = None;
                    rebuild(parsed, hooks, abort, &mut watcher, &mut watched);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn rebuild(
    parsed: &ParsedArgs,
    hooks: &mut CliHooks,
    abort: &AtomicBool,
    watcher: &mut notify::RecommendedWatcher,
    watched: &mut std::collections::BTreeSet<PathBuf>,
) {
    if abort.load(Ordering::SeqCst) {
        return;
    }
    if let Err(message) = convert_all(parsed, hooks, abort) {
        (hooks.error)(&format!("Error: {message}"));
    }
    if let Err(message) = sync_watch_roots(watcher, watched, parsed) {
        (hooks.error)(&format!("Error: {message}"));
    }
}

fn sync_watch_roots(
    watcher: &mut notify::RecommendedWatcher,
    watched: &mut std::collections::BTreeSet<PathBuf>,
    parsed: &ParsedArgs,
) -> Result<(), String> {
    let mut roots = watch_roots(parsed);
    if let Some(path) = &parsed.options {
        let loaded = read_options(path)?;
        if let Some(base) = loaded.image_base_directory {
            roots.insert(resolve_path(&base));
        }
    }
    for root in roots {
        if watched.insert(root.clone()) {
            watcher
                .watch(&root, notify::RecursiveMode::Recursive)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn watch_roots(parsed: &ParsedArgs) -> std::collections::BTreeSet<PathBuf> {
    let mut roots = std::collections::BTreeSet::new();
    if parsed.batch {
        roots.insert(resolve_path(&parsed.input));
    } else {
        roots.insert(parent_directory(&resolve_path(&parsed.input)));
    }
    for file in [&parsed.options, &parsed.reference, &parsed.patches]
        .into_iter()
        .flatten()
    {
        roots.insert(parent_directory(&resolve_path(file)));
    }
    if let Some(base) = &parsed.base_directory {
        roots.insert(resolve_path(base));
    }
    roots
}

fn parent_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn relevant_event(event: &notify::Event, parsed: &ParsedArgs) -> bool {
    if event.need_rescan() || event.paths.is_empty() {
        return true;
    }
    event
        .paths
        .iter()
        .any(|path| !ignored_artifact(path, parsed))
}

fn ignored_artifact(path: &Path, parsed: &ParsedArgs) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lowered = name.to_ascii_lowercase();
    if !lowered.ends_with(".docx") && !lowered.ends_with(".tmp") {
        return false;
    }
    let resolved = resolve_path(&path.to_string_lossy());
    let specials = [
        parsed.reference.as_deref(),
        parsed.patches.as_deref(),
        Some(parsed.input.as_str()),
    ];
    for file in specials.into_iter().flatten() {
        if resolve_path(file) == resolved {
            return false;
        }
    }
    true
}

fn io_message(path: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!(
            "ENOENT: no such file or directory, open '{}'",
            path.display()
        )
    } else {
        error.to_string()
    }
}

fn error_message(error: md_to_docx::MarkdownConversionError) -> String {
    error.message().to_string()
}

#[cfg(test)]
mod tests {
    use super::{run_cli, CliHooks};
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct Captured {
        code: i32,
        logs: Vec<String>,
        errors: Vec<String>,
        stdout: Vec<u8>,
    }

    fn run(args: &[String], stdin: Option<String>) -> Captured {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let logs_hook = Arc::clone(&logs);
        let errors_hook = Arc::clone(&errors);
        let stdout_hook = Arc::clone(&stdout);
        let mut hooks = CliHooks {
            log: Box::new(move |message| logs_hook.lock().expect("logs").push(message.to_string())),
            error: Box::new(move |message| {
                errors_hook
                    .lock()
                    .expect("errors")
                    .push(message.to_string())
            }),
            read_stdin: stdin.map(|text| {
                Box::new(move || Ok(text.clone())) as Box<dyn FnMut() -> Result<String, String>>
            }),
            write_stdout: Some(Box::new(move |data| {
                stdout_hook.lock().expect("stdout").extend_from_slice(data);
                Ok(())
            })),
        };
        let code = run_cli(args, &mut hooks, &AtomicBool::new(false));
        let logs = logs.lock().expect("logs").clone();
        let errors = errors.lock().expect("errors").clone();
        let stdout = stdout.lock().expect("stdout").clone();
        Captured {
            code,
            logs,
            errors,
            stdout,
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "md-to-docx-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn local_images_resolve_from_input_directory_and_explicit_base_dir() {
        use std::io::{Cursor, Read};

        // Valid 1x1 red and blue PNGs distinguish the default root from the override.
        const RED_PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ];
        const BLUE_PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 96, 96,
            248, 255, 31, 0, 3, 2, 1, 255, 230, 119, 11, 174, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ];
        let dir = temp_dir();
        let input_dir = dir.join("input");
        let override_dir = dir.join("override");
        fs::create_dir_all(&input_dir).unwrap();
        fs::create_dir_all(&override_dir).unwrap();
        let input = input_dir.join("report.md");
        fs::write(&input, "![Chart](chart.png)").unwrap();
        fs::write(input_dir.join("chart.png"), RED_PNG).unwrap();
        fs::write(override_dir.join("chart.png"), BLUE_PNG).unwrap();
        for (base, expected) in [(None, RED_PNG), (Some(&override_dir), BLUE_PNG)] {
            let output = dir.join("report.docx");
            let mut args = vec![
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ];
            if let Some(base) = base {
                args.extend([
                    "--base-dir".to_string(),
                    base.to_string_lossy().into_owned(),
                ]);
            }
            let captured = run(&args, None);
            assert_eq!(captured.code, 0, "{:?}", captured.errors);
            let bytes = fs::read(&output).unwrap();
            let mut package = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let mut actual = Vec::new();
            package
                .by_name("word/media/image1.png")
                .unwrap()
                .read_to_end(&mut actual)
                .unwrap();
            assert_eq!(actual, expected, "wrong image root for {base:?}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn help_goes_to_stdout_and_exits_0() {
        for flag in ["--help", "-h"] {
            let captured = run(&[flag.to_string()], None);
            assert_eq!(captured.code, 0);
            let logs = captured.logs.join("\n");
            assert!(logs.contains("Usage:"), "{logs}");
            assert!(logs.contains("--options"), "{logs}");
            assert!(captured.errors.is_empty());
        }
    }

    #[test]
    fn failure_modes_match_jest_cli_tests() {
        let cases: &[(&[&str], &str)] = &[
            (&[], "Usage:"),
            (
                &["a.md", "b.docx", "--verbose"],
                "Unknown argument: --verbose",
            ),
            (
                &["a.md", "b.docx", "--options"],
                "Missing value for --options",
            ),
            (
                &["a.md", "b.docx", "extra.md"],
                "Expected exactly 2 positional arguments",
            ),
            (&["a.md"], "Expected exactly 2 positional arguments"),
        ];
        for (argv, expected) in cases {
            let args: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
            let captured = run(&args, None);
            assert_eq!(captured.code, 1, "{argv:?}");
            let errors = captured.errors.join("\n");
            assert!(errors.contains(expected), "{argv:?} -> {errors}");
        }
    }

    #[test]
    fn fails_when_options_file_contains_plugin_configuration() {
        let dir = temp_dir();
        let input = dir.join("input.md");
        let output = dir.join("output.docx");
        let options = dir.join("options.json");
        fs::write(&input, "# Test\n\nContent.").unwrap();
        fs::write(&options, r#"{"plugins":[]}"#).unwrap();
        let captured = run(
            &[
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
                "--options".to_string(),
                options.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 1);
        let errors = captured.errors.join("\n");
        assert!(
            errors.contains("cannot be loaded from CLI JSON"),
            "{errors}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_and_non_object_options_json() {
        let dir = temp_dir();
        let input = dir.join("input.md");
        fs::write(&input, "# Test\n\nContent.").unwrap();
        for (body, expected) in [
            ("not valid json", "Invalid JSON"),
            (
                r#"["not", "an", "object"]"#,
                "Options JSON must be an object",
            ),
        ] {
            let options = dir.join("options.json");
            let output = dir.join("output.docx");
            fs::write(&options, body).unwrap();
            let captured = run(
                &[
                    input.to_string_lossy().into_owned(),
                    output.to_string_lossy().into_owned(),
                    "-o".to_string(),
                    options.to_string_lossy().into_owned(),
                ],
                None,
            );
            assert_eq!(captured.code, 1, "{body}");
            let errors = captured.errors.join("\n");
            assert!(errors.contains(expected), "{body} -> {errors}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_input_mentions_enoent() {
        let dir = temp_dir();
        let captured = run(
            &[
                dir.join("missing.md").to_string_lossy().into_owned(),
                dir.join("output.docx").to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 1);
        let errors = captured.errors.join("\n");
        assert!(
            errors.contains("ENOENT") || errors.to_ascii_lowercase().contains("no such file"),
            "{errors}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_a_file_and_creates_nested_directories() {
        let dir = temp_dir();
        let input = dir.join("input.md");
        let output = dir.join("nested").join("deep").join("output.docx");
        fs::write(&input, "# Nested\n\nContent.").unwrap();
        let captured = run(
            &[
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 0, "{:?}", captured.errors);
        assert!(captured.logs.join("\n").contains("DOCX created at:"));
        let metadata = fs::metadata(&output).unwrap();
        assert!(metadata.len() > 0);
        assert!(fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_style_and_multi_section_options() {
        let dir = temp_dir();
        let input = dir.join("input.md");
        fs::write(&input, "# Styled CLI Test\n\nContent.").unwrap();
        let simple = dir.join("options.json");
        fs::write(
            &simple,
            r#"{"documentType":"report","style":{"heading1Alignment":"CENTER","paragraphAlignment":"JUSTIFIED"}}"#,
        )
        .unwrap();
        let long_out = dir.join("long.docx");
        let captured = run(
            &[
                input.to_string_lossy().into_owned(),
                long_out.to_string_lossy().into_owned(),
                "--options".to_string(),
                simple.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 0, "{:?}", captured.errors);
        assert!(fs::metadata(&long_out).unwrap().len() > 0);

        let short_out = dir.join("short.docx");
        let captured = run(
            &[
                input.to_string_lossy().into_owned(),
                short_out.to_string_lossy().into_owned(),
                "-o".to_string(),
                simple.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 0, "{:?}", captured.errors);

        let multi = dir.join("multi.json");
        fs::write(
            &multi,
            r##"{"template":{"page":{"margin":{"top":1440,"right":1080,"bottom":1440,"left":1080}},"pageNumbering":{"display":"current","alignment":"CENTER"}},"sections":[{"markdown":"# Cover\n\nPrepared by CLI","footers":{"default":null},"pageNumbering":{"display":"none"},"style":{"paragraphAlignment":"CENTER"}},{"markdown":"# Body\n\nMain content starts here.\n\n1. One\n2. Two","headers":{"default":{"text":"Main Section","alignment":"RIGHT"}},"pageNumbering":{"start":1,"formatType":"decimal"}}]}"##,
        )
        .unwrap();
        let multi_out = dir.join("multi.docx");
        let captured = run(
            &[
                input.to_string_lossy().into_owned(),
                multi_out.to_string_lossy().into_owned(),
                "--options".to_string(),
                multi.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 0, "{:?}", captured.errors);
        assert!(fs::metadata(&multi_out).unwrap().len() > 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_stdout_keeps_status_off_stdout() {
        let captured = run(
            &["-".to_string(), "-".to_string()],
            Some("# Piped".to_string()),
        );
        assert_eq!(captured.code, 0, "{:?}", captured.errors);
        assert!(captured.logs.is_empty(), "{:?}", captured.logs);
        assert!(captured.stdout.starts_with(b"PK"));
    }

    #[test]
    fn batch_sorts_like_locale_compare_and_continues_after_failure() {
        let dir = temp_dir();
        let input = dir.join("input");
        let output = dir.join("output");
        fs::create_dir_all(input.join("nested")).unwrap();
        fs::write(input.join("b.md"), "# Bee").unwrap();
        fs::write(input.join("a.md"), "# Aye").unwrap();
        fs::write(input.join("nested").join("c.md"), "# Sea").unwrap();
        fs::write(input.join("notes.txt"), "skip").unwrap();
        fs::write(input.join("long.md"), "x".repeat(20)).unwrap();
        let options = dir.join("limits.json");
        fs::write(&options, r#"{"maxInputLength":5}"#).unwrap();
        let captured = run(
            &[
                "--batch".to_string(),
                input.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
                "--options".to_string(),
                options.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 1, "{:?}", captured.errors);
        let errors = captured.errors.join("\n");
        assert!(errors.contains("long.md"), "{errors}");
        assert!(
            errors.contains("One or more batch conversions failed"),
            "{errors}"
        );
        assert!(output.join("a.docx").is_file());
        assert!(output.join("b.docx").is_file());
        assert!(output.join("nested").join("c.docx").is_file());
        assert!(!output.join("notes.docx").exists());
        assert!(!output.join("long.docx").exists());
        let logs = captured.logs.join("\n");
        let aye = logs.find("a.docx").expect("a");
        let bee = logs.find("b.docx").expect("b");
        let sea = logs.find("c.docx").expect("c");
        assert!(aye < bee && bee < sea, "{logs}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_directories_must_differ() {
        let dir = temp_dir();
        let captured = run(
            &[
                "--batch".to_string(),
                dir.to_string_lossy().into_owned(),
                dir.to_string_lossy().into_owned(),
            ],
            None,
        );
        assert_eq!(captured.code, 1);
        assert!(captured
            .errors
            .join("\n")
            .contains("Batch input and output directories must differ"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_rebuilds_after_debounce_and_stops() {
        let dir = temp_dir();
        let input = dir.join("input.md");
        let output = dir.join("output.docx");
        fs::write(&input, "# Before").unwrap();
        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_hook = Arc::clone(&logs);
        let abort = Arc::new(AtomicBool::new(false));
        let abort_thread = Arc::clone(&abort);
        let args = vec![
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            "--watch".to_string(),
        ];
        let handle = std::thread::spawn(move || {
            let mut hooks = CliHooks {
                log: Box::new(move |message| {
                    logs_hook.lock().expect("logs").push(message.to_string())
                }),
                error: Box::new(|_| {}),
                read_stdin: None,
                write_stdout: None,
            };
            run_cli(&args, &mut hooks, &abort_thread)
        });
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            if logs
                .lock()
                .expect("logs")
                .iter()
                .any(|message| message.contains("Watching"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let watching = logs
            .lock()
            .expect("logs")
            .iter()
            .any(|message| message.contains("Watching"));
        if !watching {
            abort.store(true, Ordering::SeqCst);
            let _ = handle.join();
            panic!("watch did not start: {:?}", logs.lock().unwrap());
        }
        fs::write(&input, "# After").unwrap();
        let changed = Instant::now();
        while changed.elapsed() < Duration::from_secs(8) {
            let created = logs
                .lock()
                .expect("logs")
                .iter()
                .filter(|message| message.contains("created"))
                .count();
            if created >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        abort.store(true, Ordering::SeqCst);
        let code = handle.join().expect("watch thread");
        assert_eq!(code, 0);
        let created = logs
            .lock()
            .expect("logs")
            .iter()
            .filter(|message| message.contains("created"))
            .count();
        assert!(created >= 2, "{:?}", logs.lock().unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
}
