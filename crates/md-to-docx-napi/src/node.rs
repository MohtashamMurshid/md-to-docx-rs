//! Native conversion runs on the libuv worker pool. JavaScript selects the output type.
#![cfg_attr(test, allow(dead_code))]

use napi::bindgen_prelude::{AsyncTask, Buffer, Either, Uint8Array};
use napi::{Env, Error, Result, Status, Task};
use napi_derive::napi;

pub struct ConversionTask {
    markdown: String,
    reference: Option<Vec<u8>>,
    patches: Option<String>,
    options: Option<String>,
}

impl Task for ConversionTask {
    type Output = Vec<u8>;
    type JsValue = Buffer;

    fn compute(&mut self) -> Result<Self::Output> {
        let result = match (&self.reference, &self.patches) {
            (Some(reference), Some(patches)) => {
                crate::patch_markdown_in_docx(reference, patches, self.options.as_deref())
            }
            (Some(reference), None) => crate::convert_markdown_with_reference_docx(
                &self.markdown,
                reference,
                self.options.as_deref(),
            ),
            _ => crate::convert_markdown_to_buffer(&self.markdown, self.options.as_deref()),
        };
        result.map_err(|error| {
            Error::new(
                Status::GenericFailure,
                format!(
                    "MD_TO_DOCX_ERROR:{}",
                    serde_json::json!({"message": error.message(), "context": error.context_json()})
                ),
            )
        })
    }

    fn resolve(&mut self, _env: Env, output: Vec<u8>) -> Result<Buffer> {
        Ok(output.into())
    }
}

fn input_bytes(value: Either<Buffer, Uint8Array>) -> Vec<u8> {
    match value {
        Either::A(buffer) => buffer.as_ref().to_vec(),
        Either::B(array) => array.as_ref().to_vec(),
    }
}

#[napi]
pub fn convert_markdown_to_docx(
    markdown: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: None,
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn convert_markdown_to_array_buffer(
    markdown: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: None,
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn convert_markdown_to_buffer(
    markdown: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: None,
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn convert_markdown_with_reference_docx(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: Some(input_bytes(reference_docx)),
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn convert_markdown_with_reference_docx_to_array_buffer(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: Some(input_bytes(reference_docx)),
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn convert_markdown_with_reference_docx_to_buffer(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown,
        reference: Some(input_bytes(reference_docx)),
        patches: None,
        options: options_json,
    })
}

#[napi]
pub fn patch_markdown_in_docx(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown: String::new(),
        reference: Some(input_bytes(reference_docx)),
        patches: Some(patches_json),
        options: options_json,
    })
}

#[napi]
pub fn patch_markdown_in_docx_to_array_buffer(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown: String::new(),
        reference: Some(input_bytes(reference_docx)),
        patches: Some(patches_json),
        options: options_json,
    })
}

#[napi]
pub fn patch_markdown_in_docx_to_buffer(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> AsyncTask<ConversionTask> {
    AsyncTask::new(ConversionTask {
        markdown: String::new(),
        reference: Some(input_bytes(reference_docx)),
        patches: Some(patches_json),
        options: options_json,
    })
}

// Callback conversions use a dedicated thread so callbacks can themselves await
// filesystem work without exhausting the libuv pool used by plain conversions.
use napi::bindgen_prelude::{Function, Object, Promise};
use napi::threadsafe_function::ThreadsafeFunction;
use std::sync::Arc;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct WakeThread(std::thread::Thread);
    impl std::task::Wake for WakeThread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(WakeThread(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(result) => return result,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

type Callback = ThreadsafeFunction<String, Promise<String>, String, Status, false>;

fn call_callback(
    callback: &Callback,
    input: serde_json::Value,
) -> std::result::Result<serde_json::Value, md_to_docx::MarkdownConversionError> {
    let result = block_on(async { callback.call_async_catch(input.to_string()).await?.await })
        .map_err(|error| md_to_docx::MarkdownConversionError::new(error.reason))?;
    serde_json::from_str(&result).map_err(|error| {
        md_to_docx::MarkdownConversionError::new(format!("Invalid callback result: {error}"))
    })
}

fn callback_bytes(
    value: &serde_json::Value,
) -> std::result::Result<Vec<u8>, md_to_docx::MarkdownConversionError> {
    serde_json::from_value(value.get("data").cloned().unwrap_or_default()).map_err(|_| {
        md_to_docx::MarkdownConversionError::new("Renderer/resolver must return image bytes")
    })
}

fn optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn install_callbacks(
    options: &mut md_to_docx::ConvertOptions,
    callback: Arc<Callback>,
    enabled: &serde_json::Value,
) {
    if enabled["image"].as_bool() == Some(true) {
        let callback = callback.clone();
        options
            .image_handling
            .get_or_insert_with(Default::default)
            .resolve = Some(Arc::new(move |source, context| {
            let value = call_callback(
                &callback,
                serde_json::json!({"kind":"image", "source":source, "maxBytes":context.max_bytes}),
            )?;
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(md_to_docx::ResolvedImage {
                bytes: callback_bytes(&value)?,
                content_type: optional_string(&value, "contentType"),
            }))
        }));
    }
    if enabled["chart"].as_bool() == Some(true) {
        let callback = callback.clone();
        let mut images = options.image_handling.clone().unwrap_or_default();
        images.data_urls_enabled = true;
        options
            .chart_rendering
            .get_or_insert_with(Default::default)
            .renderer = Some(md_to_docx::options::ChartRenderer::new(
            move |source, width, height| {
                let definition: serde_json::Value = serde_json::from_str(source)
                    .map_err(|error| md_to_docx::MarkdownConversionError::new(error.to_string()))?;
                let value = call_callback(
                    &callback,
                    serde_json::json!({"kind":"chart", "definition":definition, "width":width, "height":height}),
                )?;
                if let Some(source) = value["source"].as_str() {
                    return md_to_docx::load_markdown_image(source, "Chart", None, &images)
                        .map(|image| image.bytes);
                }
                callback_bytes(&value)
            },
        ));
    }
    if enabled["highlight"].as_bool() == Some(true) {
        let callback = callback.clone();
        options
            .code_highlighting
            .get_or_insert_with(Default::default)
            .tokenizer = Some(md_to_docx::options::HighlightTokenizer::new(
            move |code, language| {
                let value = call_callback(
                    &callback,
                    serde_json::json!({"kind":"highlight", "code":code, "language":language}),
                )?;
                let Some(items) = value.as_array() else {
                    return Ok(None);
                };
                let spans = items
                    .iter()
                    .map(|item| md_to_docx::highlight::HighlightSpan {
                        text: item["text"].as_str().unwrap_or("").to_owned(),
                        color: item["color"].as_str().map(str::to_owned),
                    })
                    .collect();
                Ok(Some(spans))
            },
        ));
    }
    if enabled["mermaid"].as_bool() == Some(true) {
        options
            .mermaid_rendering
            .get_or_insert_with(Default::default)
            .render = Some(md_to_docx::options::MermaidRenderer::new(move |input| {
            let value = call_callback(
                &callback,
                serde_json::json!({"kind":"mermaid", "code":input.code, "meta":input.meta}),
            )?;
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(md_to_docx::options::MermaidRenderOutput {
                bytes: callback_bytes(&value)?,
                content_type: optional_string(&value, "contentType")
                    .unwrap_or_else(|| "image/png".to_owned()),
                width_px: value["width"]
                    .as_u64()
                    .and_then(|width| u32::try_from(width).ok()),
                height_px: value["height"]
                    .as_u64()
                    .and_then(|height| u32::try_from(height).ok()),
                alt: optional_string(&value, "altText").unwrap_or_default(),
                title: optional_string(&value, "title"),
            }))
        }));
    }
}

#[napi]
pub fn convert_with_callbacks<'env>(
    env: &'env Env,
    markdown: String,
    reference_docx: Option<Buffer>,
    patches_json: Option<String>,
    options_json: String,
    callbacks_json: String,
    callback: Function<'env, String, Promise<String>>,
) -> Result<Object<'env>> {
    let callback = Arc::new(
        callback
            .build_threadsafe_function::<String>()
            .callee_handled::<false>()
            .build()?,
    );
    let reference = reference_docx.map(|buffer| buffer.as_ref().to_vec());
    let (deferred, promise) = env.create_deferred::<Buffer, _>()?;
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut loaded = crate::options_json::load_options_json(&options_json, "options")
                .map_err(md_to_docx::MarkdownConversionError::new)?;
            let enabled = serde_json::from_str(&callbacks_json).map_err(|error| {
                md_to_docx::MarkdownConversionError::new(format!(
                    "Invalid callback configuration: {error}"
                ))
            })?;
            install_callbacks(&mut loaded.convert, callback, &enabled);
            match (reference, patches_json) {
                (Some(reference), Some(patches)) => {
                    loaded.patch.convert = loaded.convert;
                    let patches = crate::options_json::load_patches_json(&patches)
                        .map_err(md_to_docx::MarkdownConversionError::new)?;
                    md_to_docx::patch_markdown(&reference, &patches, &loaded.patch)
                }
                (Some(reference), None) => md_to_docx::convert_with_reference(
                    &markdown,
                    &reference,
                    &md_to_docx::ReferenceDocxGenerationOptions {
                        reference: loaded.reference,
                        aborted: loaded.aborted,
                        convert: loaded.convert,
                    },
                ),
                _ => md_to_docx::convert_with_options(&markdown, &loaded.convert),
            }
        }));
        match result {
            Ok(Ok(bytes)) => deferred.resolve(move |_env| Ok(Buffer::from(bytes))),
            Ok(Err(error)) => deferred.reject(Error::new(
                Status::GenericFailure,
                format!(
                    "MD_TO_DOCX_ERROR:{}",
                    serde_json::json!({"message":error.message(), "context":error.context_json()})
                ),
            )),
            Err(_) => deferred.reject(Error::new(
                Status::GenericFailure,
                "Native conversion worker panicked",
            )),
        }
    });
    Ok(promise)
}
