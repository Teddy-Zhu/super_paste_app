# 剪贴板历史记录清理器

## 概述

剪贴板历史记录清理器是一个自动管理剪贴板历史记录的组件，根据用户配置的最大保存时间自动清理过期的历史记录。

## 功能特性

- **自动清理**: 根据配置的最大保存时间自动清理过期记录
- **配置管理**: 支持动态更新清理配置
- **统计信息**: 提供详细的清理统计信息
- **事务安全**: 使用数据库事务确保清理操作的原子性

## 配置说明

### SETTINGS_MAX_KEEP_TIME

- **类型**: `number`
- **默认值**: `-1`
- **说明**: 历史记录的最大保存时间（天）
  - `-1`: 不限制时间，不会自动清理
  - `1-365`: 保存指定天数，超过时间的记录会被自动清理

## 使用方法

### 1. 设置最大保存时间

在应用的设置界面中：

1. 打开"常规设置"
2. 找到"历史记录保存时间"设置项
3. 输入保存天数（1-365），或输入 -1 表示不限制
4. 配置会自动保存并生效

### 2. 自动清理

当设置了有效的保存时间后，清理器会：

1. 立即执行一次清理
2. 每小时自动检查并清理过期记录
3. 在应用启动时自动启动清理器

## 技术实现

### 核心类

#### ClipboardHistoryCleaner

主要的清理器类，提供以下功能：

- `startAutoCleanup()`: 启动自动清理
- `stopAutoCleanup()`: 停止自动清理
- `performCleanup()`: 执行清理操作
- `updateConfigFromStore()`: 从配置更新清理器设置

### 数据库支持

#### 新增方法

在 `ClipboardDatabase` 类中新增了以下方法：

- `deleteRecordsByTimestamp(beforeTimestamp)`: 删除指定时间戳之前的记录
- `countRecordsByTimestamp(beforeTimestamp)`: 统计指定时间戳之前的记录数量



### 类型定义

#### CleanupStats

```typescript
interface CleanupStats {
  totalRecords: number     // 清理前总记录数
  expiredRecords: number   // 过期记录数
  deletedRecords: number   // 实际删除的记录数
  cleanupTime: number      // 清理耗时（毫秒）
}
```



#### CleanupConfig

```typescript
interface CleanupConfig {
  maxKeepTime: number        // 最大保存时间（天）
  cleanupInterval: number    // 清理检查间隔（毫秒）
  enableAutoCleanup: boolean // 是否启用自动清理
}
```

## 集成说明

### 主进程集成

1. 在 `clipboard-manager.ts` 中集成清理器
2. 在应用初始化时启动清理器
3. 在配置更新时同步更新清理器配置
4. 在应用退出时正确清理资源

### 前端集成

1. 在设置 store 中添加 `maxKeepTime` 配置
2. 在设置界面中添加保存时间设置项

## 注意事项

1. **性能考虑**: 清理操作使用数据库事务，确保数据一致性
2. **用户体验**: 自动清理在后台静默执行，不影响用户操作
3. **配置同步**: 配置更改会立即生效，无需重启应用
4. **资源管理**: 应用退出时会正确停止清理器，避免资源泄漏

### 使用方法

1. **设置保存时间**：在设置界面中输入保存天数（1-365），或输入 -1 表示不限制
2. **自动清理**：设置有效时间后，系统每小时自动清理过期记录

## 故障排除

### 清理器未启动

检查 `SETTINGS_MAX_KEEP_TIME` 配置是否为 -1，如果是则清理器不会启动。

### 清理操作失败

检查控制台日志，可能的原因：
- 数据库连接问题
- 权限问题
- 磁盘空间不足

### 配置不生效

确保配置更新后调用了 `updateMaxKeepTimeFromStore()` 方法。
