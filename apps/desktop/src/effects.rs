use std::{collections::BTreeSet, sync::Arc};
use super_paste_core::{Error, Result, configuration::{ConfigurationEffects, PreparedConfigurationEffects, RuntimeConfiguration}, mcp::{McpService, PreparedMcpChange}, ocr::OcrService};
use super_paste_format::ConfigField;
use super_paste_platform::autostart::{AutoStart, AutoStartChange};
use super_paste_platform::shortcut::{ShortcutService, PreparedShortcutChange};

pub struct DesktopEffects {
    pub ocr: Arc<OcrService>,
    pub auto_start: Arc<AutoStart>,
    pub mcp: Arc<McpService>,
    pub shortcuts: Arc<ShortcutService>,
}
pub struct PreparedEffects { auto_start: Option<AutoStartChange>, mcp: Option<PreparedMcpChange>, shortcut: Option<PreparedShortcutChange> }

impl ConfigurationEffects for DesktopEffects {
    async fn prepare(&self, _: RuntimeConfiguration, after: RuntimeConfiguration, fields: BTreeSet<ConfigField>) -> Result<impl PreparedConfigurationEffects + 'static> {
        if fields.iter().any(|field| matches!(field, ConfigField::EnableLocalOcr | ConfigField::OcrMode | ConfigField::OcrLocalModel)) { self.ocr.validate_configuration(&after.value.settings).await?; }
        let auto_start = if fields.contains(&ConfigField::AutoStart) {
            Some(self.auto_start.prepare(after.value.settings.auto_start).await?)
        } else { None };
        let shortcut = if fields.contains(&ConfigField::GlobalShortcut) { Some(self.shortcuts.prepare(&after.value.settings.global_shortcut).await?) } else { None };
        let mcp = if McpService::affects(&fields) { Some(self.mcp.prepare(after, &fields).await?) } else { None };
        Ok(PreparedEffects { auto_start, mcp, shortcut })
    }
}
impl PreparedConfigurationEffects for PreparedEffects {
    async fn apply(&mut self) -> Result<()> {
        if let Some(change) = &mut self.auto_start { change.apply().await?; }
        if let Some(change) = &mut self.shortcut { change.apply().await?; }
        if let Some(change) = &mut self.mcp { change.apply().await?; }
        Ok(())
    }
    async fn rollback(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        if let Some(change) = &mut self.mcp { if let Err(error) = change.rollback().await { errors.push(error.to_string()); } }
        if let Some(change) = &mut self.shortcut { if let Err(error) = change.rollback().await { errors.push(error.to_string()); } }
        if let Some(change) = &mut self.auto_start { if let Err(error) = change.rollback().await { errors.push(error.to_string()); } }
        if errors.is_empty() { Ok(()) } else { Err(Error::Task(errors.join("；").into())) }
    }
}
