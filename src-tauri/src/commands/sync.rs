use super::{Runtime, argument, data};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
) -> Result<Value, String> {
    match channel {
        "scheduled-sync:test-connection" => {
            let result = runtime.sync.test_connection(args.first().cloned()).await;
            if result.success {
                Ok(json!(true))
            } else {
                Ok(json!({"success":false,"error":result.message}))
            }
        }
        "scheduled-sync:manual-sync" => {
            serde_json::to_value(runtime.sync.manual_sync().await).map_err(|e| e.to_string())
        }
        "scheduled-sync:manual-sync-with-config" => serde_json::to_value(
            runtime
                .sync
                .manual_sync_with_config(argument(args, 0)?)
                .await,
        )
        .map_err(|e| e.to_string()),
        "scheduled-sync:get-status" => data(runtime.sync.status().await),
        _ => Err(format!("未由 sync 处理的命令: {channel}")),
    }
}
