use serde_json::{Value, json};

use super::{Storage, clipboard, statistics, tags, usage};

pub(super) async fn invoke(
    storage: &Storage,
    channel: &str,
    args: &[Value],
) -> Result<Value, String> {
    let first = args.first().cloned().unwrap_or(Value::Null);
    match channel {
        "clipboard-get-history" => clipboard::query_history(storage, &first).await,
        "clipboard-delete-item" => {
            let id = first
                .as_str()
                .ok_or_else(|| "历史记录 ID 无效".to_owned())?;
            let success = clipboard::delete_clipboard(storage, id).await?;
            let deleted_ids = if success {
                vec![id.to_owned()]
            } else {
                Vec::new()
            };
            Ok(
                json!({"success":success,"message":if success {"剪贴板项已删除"} else {"删除失败，未找到指定项目"},"deletedIds":deleted_ids}),
            )
        }
        "clipboard-clear-history" => {
            let deleted_ids = clipboard::clear_clipboard(storage).await?;
            Ok(json!({"success":true,"message":"剪贴板历史已清空","deletedIds":deleted_ids}))
        }
        "get-all-tags" => Ok(json!({"success":true,"data":tags::get_all_tags(storage).await?})),
        "create-tag" => match tags::create_tag(storage, &first).await {
            Ok(tag_id) => {
                Ok(json!({"success":true,"data":{"tagId":tag_id},"message":"标签创建成功"}))
            }
            Err(error) => Ok(json!({"success":false,"error":error})),
        },
        "update-tag" => {
            let tag_id = first
                .get("tagId")
                .and_then(Value::as_str)
                .ok_or_else(|| "标签 ID 无效".to_owned())?;
            let updates = first.get("updates").cloned().unwrap_or_else(|| json!({}));
            let success = tags::update_tag(storage, tag_id, &updates).await?;
            Ok(
                json!({"success":success,"message":if success {"标签更新成功"} else {"更新标签失败"}}),
            )
        }
        "delete-tag" => {
            let tag_id = first.as_str().ok_or_else(|| "标签 ID 无效".to_owned())?;
            let success = tags::delete_tag(storage, tag_id).await?;
            Ok(
                json!({"success":success,"message":if success {"标签删除成功"} else {"删除标签失败"}}),
            )
        }
        "get-tag-stats" => Ok(json!({"success":true,"data":tags::get_tag_stats(storage).await?})),
        "bind-tag-to-record" | "unbind-tag-from-record" => {
            let content_id = first
                .get("contentId")
                .and_then(Value::as_str)
                .ok_or_else(|| "剪贴板记录 ID 无效".to_owned())?;
            let tag_id = first
                .get("tagId")
                .and_then(Value::as_str)
                .ok_or_else(|| "标签 ID 无效".to_owned())?;
            let success = if channel == "bind-tag-to-record" {
                tags::bind_tag(storage, content_id, tag_id).await?
            } else {
                tags::unbind_tag(storage, content_id, tag_id).await?
            };
            let message = if channel == "bind-tag-to-record" {
                "标签绑定成功"
            } else {
                "标签解绑成功"
            };
            Ok(json!({"success":success,"message":if success {message} else {"标签操作失败"}}))
        }
        "statistics:get-copy" => statistics::get_copy_stats(storage, &first).await,
        "statistics:clear-copy" => {
            statistics::clear_copy_stats(storage).await?;
            Ok(json!({"success":true}))
        }
        "ocr:get-usage" => usage::get_ai_usage_stats(storage, &first).await,
        "ocr:clear-usage" => usage::clear_ai_usage(storage).await,
        _ => Err(format!("未由 Storage 处理的通道: {channel}")),
    }
}
