use super::{Runtime, argument, data, ok, record_id};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
) -> Result<Value, String> {
    match channel {
        "ocr:list-models" => data(runtime.ocr.list_models().await?),
        "ocr:download-model" => {
            runtime.ocr.download_model(&record_id(args)?).await?;
            Ok(ok())
        }
        "ocr:cancel-download" => {
            runtime.ocr.cancel_download(&record_id(args)?).await?;
            Ok(ok())
        }
        "ocr:delete-model" => {
            runtime.ocr.delete_model(&record_id(args)?).await?;
            Ok(ok())
        }
        "ocr:redownload-model" => {
            runtime.ocr.redownload_model(&record_id(args)?).await?;
            Ok(ok())
        }
        "ocr:ai-list-models" | "ocr:ai-test-connection" => {
            let request: Value = argument(args, 0)?;
            let provider = request["provider"].as_str().ok_or("缺少 AI 提供商")?;
            let candidate = request["config"].as_object().ok_or("AI 配置格式无效")?;
            if candidate
                .keys()
                .any(|key| !matches!(key.as_str(), "baseUrl" | "model" | "api"))
            {
                return Err("AI 测试仅接受非敏感连接参数，密钥须先通过配置保存".into());
            }
            let mut config = runtime.config.runtime_ai_config(provider).await?;
            let target = config.as_object_mut().ok_or("已保存的 AI 配置格式无效")?;
            for (key, value) in candidate {
                target.insert(key.clone(), value.clone());
            }
            if channel == "ocr:ai-list-models" {
                data(runtime.ocr.ai_list_models(provider, &config).await?)
            } else {
                data(runtime.ocr.ai_test_connection(provider, &config).await?)
            }
        }
        _ => Err(format!("未由 ocr 处理的命令: {channel}")),
    }
}
