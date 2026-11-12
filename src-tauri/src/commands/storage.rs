use super::{Runtime, emit};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
) -> Result<Value, String> {
    let result = runtime.storage.invoke(channel, args).await?;
    if result["success"].as_bool() == Some(true) {
        match channel {
            "clipboard-delete-item" | "clipboard-clear-history" => {
                let ids = result["deletedIds"]
                    .as_array()
                    .ok_or("删除操作未返回记录清单")?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "删除清单格式无效".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                runtime.ocr.cancel_records(&ids).await?;
                emit(
                    runtime,
                    "clipboard-history-updated",
                    json!({"reason":"delete","ids":ids}),
                );
            }
            "create-tag"
            | "update-tag"
            | "delete-tag"
            | "bind-tag-to-record"
            | "unbind-tag-from-record" => {
                emit(runtime, "tags-updated", Value::Null);
                emit(
                    runtime,
                    "clipboard-history-updated",
                    json!({"reason":"tags"}),
                );
            }
            "statistics:clear-copy" => emit(runtime, "statistics:copy-updated", Value::Null),
            "ocr:clear-usage" => emit(runtime, "ocr:usage-updated", Value::Null),
            _ => {}
        }
    }
    Ok(result)
}
