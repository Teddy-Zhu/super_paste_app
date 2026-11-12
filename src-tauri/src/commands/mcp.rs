use super::{Runtime, argument, data};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
) -> Result<Value, String> {
    match channel {
        "mcp:get-state" => data(runtime.mcp_config.state().await?),
        "mcp:save-settings" => {
            let settings = argument(args, 0)?;
            data(runtime.mcp_config.save(settings).await?)
        }
        "mcp:generate-key" => data(crate::core::mcp_config::generate_key()),
        _ => Err(format!("未由 mcp 处理的命令: {channel}")),
    }
}
