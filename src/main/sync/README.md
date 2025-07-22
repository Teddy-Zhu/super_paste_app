# 同步功能使用文档

本文档介绍了如何使用 Super Paste App 的数据库和配置文件同步功能。

## 架构概述

同步功能采用了抽象设计模式，包含以下核心组件：

1. **types.ts** - 所有类型定义的中心文件，避免类型重复
2. **BaseSyncProvider** - 抽象基类，定义了所有同步提供者的通用接口
3. **WebDAVSyncProvider** - WebDAV 同步提供者的具体实现
4. **SyncService** - 同步服务管理器，负责管理同步流程和配置
5. **sync-integration.ts** - 提供集成示例和 IPC 处理器

## 功能特性

- 📁 支持数据库文件和配置文件同步
- 🔄 支持三种同步方向：上传、下载、双向同步
- ⏰ 支持自动定时同步
- 🔌 可扩展的同步提供者架构
- 📊 完整的同步状态和进度反馈
- 🔒 支持加密存储的配置管理
- 🎯 统一的类型定义系统

## 快速开始

### 1. 导入类型和服务

```typescript
import { 
  syncService, 
  SyncProviderType, 
  SyncDirection,
  type WebDAVConfig 
} from '../sync'

// 初始化同步服务
await syncService.initialize()
```

### 2. 配置 WebDAV 同步

```typescript
import { SyncDirection, type WebDAVConfig } from '../sync'

const webdavConfig: WebDAVConfig = {
  enabled: true,
  autoSync: true,
  syncInterval: 30, // 30分钟
  syncDirection: SyncDirection.BIDIRECTIONAL,
  serverUrl: 'https://your-webdav-server.com',
  username: 'your-username',
  password: 'your-password',
  remotePath: '/super-paste'
}

// 配置 WebDAV 同步提供者
const success = await syncService.configureSyncProvider(
  SyncProviderType.WEBDAV, 
  webdavConfig
)

if (success) {
  console.log('WebDAV 同步配置成功')
} else {
  console.error('WebDAV 同步配置失败')
}
```

### 3. 测试连接

```typescript
const connectionOk = await syncService.testConnection()
if (connectionOk) {
  console.log('同步服务器连接正常')
} else {
  console.error('同步服务器连接失败')
}
```

### 4. 执行同步

```typescript
// 手动执行同步
const result = await syncService.sync()

if (result.success) {
  console.log('同步成功:', result.message)
  console.log('上传的文件:', result.uploadedFiles)
  console.log('下载的文件:', result.downloadedFiles)
} else {
  console.error('同步失败:', result.message)
  console.error('错误详情:', result.errors)
}
```

### 5. 监听同步事件

```typescript
// 监听同步状态变化
syncService.addSyncEventListener('status-changed', (event) => {
  console.log('同步状态变化:', event.data.status)
})

// 监听文件同步事件
syncService.addSyncEventListener('file-synced', (event) => {
  console.log('文件同步完成:', event.data)
})
```

## 高级用法

### 自定义同步提供者

如果需要添加其他同步方式（如 FTP、SFTP、云存储等），可以继承 `BaseSyncProvider`：

```typescript
import { BaseSyncProvider } from '../sync'
import type { 
  SyncConfig, 
  SyncResult, 
  SyncFileInfo 
} from '../sync'

interface CustomSyncConfig extends SyncConfig {
  customOption: string
}

class CustomSyncProvider extends BaseSyncProvider {
  constructor(config: CustomSyncConfig) {
    super(config)
  }

  async initialize(): Promise<boolean> {
    // 实现初始化逻辑
    return true
  }

  async testConnection(): Promise<boolean> {
    // 实现连接测试逻辑
    return true
  }

  async uploadFile(localPath: string, remotePath: string): Promise<boolean> {
    // 实现文件上传逻辑
    return true
  }

  async downloadFile(remotePath: string, localPath: string): Promise<boolean> {
    // 实现文件下载逻辑
    return true
  }

  // 实现其他必需的方法...
  
  protected async performSync(): Promise<SyncResult> {
    // 实现具体的同步逻辑
    return {
      success: true,
      message: '同步完成'
    }
  }
}
```

### 管理自动同步

```typescript
// 启动自动同步
syncService.startAutoSync()

// 停止自动同步
syncService.stopAutoSync()

// 获取同步状态
const status = syncService.getSyncStatus()
console.log('当前同步状态:', status)
```

### 配置管理

同步配置会自动保存到加密存储中，应用重启后会自动加载。

```typescript
// 禁用同步（会停止自动同步并清除配置）
syncService.disableSync()
```

## 同步的文件

目前同步服务会处理以下文件：

1. **数据库文件** (`clipboard-history.db`)
   - 包含所有剪贴板历史记录
   - 包含标签和应用信息
   
2. **配置文件** (`super-paste-config.json`)
   - 应用设置和偏好
   - 用户自定义配置

## 同步方向说明

- **UPLOAD**: 仅上传本地文件到远程服务器
- **DOWNLOAD**: 仅从远程服务器下载文件到本地
- **BIDIRECTIONAL**: 双向同步，根据文件修改时间决定同步方向

## 错误处理

同步过程中的所有错误都会被捕获并记录在日志中。可以通过同步结果的 `errors` 字段获取详细的错误信息。

```typescript
const result = await syncService.sync()
if (!result.success && result.errors) {
  result.errors.forEach(error => {
    console.error('同步错误:', error)
  })
}
```

## 注意事项

1. WebDAV 服务器需要支持文件上传和下载权限
2. 确保网络连接稳定，同步过程中断可能导致数据不一致
3. 建议在首次配置时先测试连接，确保服务器配置正确
4. 自动同步间隔建议不要设置太短，避免频繁的网络请求
5. 双向同步时，如果本地和远程文件同时修改，以修改时间较新的为准

## 扩展计划

未来可能添加的同步提供者：

- FTP/SFTP 同步
- 各大云存储服务（Google Drive、Dropbox、OneDrive）
- Git 仓库同步
- 局域网同步
