<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'

interface Props {
  settingsStore: any
}

const props = defineProps<Props>()

interface OCRSettingsForm {
  enableLocalOCR: boolean
  ocrMode: 'local' | 'remote'
  ocrRemoteUrl: string
  ocrRemoteExtraParams: string
  ocrRemoteImageFieldName: string
  ocrRemoteRequestFormat: 'json' | 'form'
  ocrRemoteResultPath: string
}

const form = reactive<OCRSettingsForm>({
  enableLocalOCR: false,
  ocrMode: 'local',
  ocrRemoteUrl: '',
  ocrRemoteExtraParams: '{}',
  ocrRemoteImageFieldName: 'image',
  ocrRemoteRequestFormat: 'json',
  ocrRemoteResultPath: 'data.result'
})

const saveMessage = ref('')
const saveError = ref('')
const savedSnapshot = ref('')

const readStoreConfig = (): OCRSettingsForm => ({
  enableLocalOCR: Boolean(props.settingsStore.enableLocalOCR),
  ocrMode: props.settingsStore.ocrMode === 'remote' ? 'remote' : 'local',
  ocrRemoteUrl: String(props.settingsStore.ocrRemoteUrl || ''),
  ocrRemoteExtraParams: String(props.settingsStore.ocrRemoteExtraParams || '{}'),
  ocrRemoteImageFieldName: String(props.settingsStore.ocrRemoteImageFieldName || 'image'),
  ocrRemoteRequestFormat:
    props.settingsStore.ocrRemoteRequestFormat === 'form' ? 'form' : 'json',
  ocrRemoteResultPath: String(props.settingsStore.ocrRemoteResultPath || 'data.result')
})

const buildSnapshot = (config: OCRSettingsForm): string =>
  JSON.stringify({
    enableLocalOCR: config.enableLocalOCR,
    ocrMode: config.ocrMode,
    ocrRemoteUrl: config.ocrRemoteUrl.trim(),
    ocrRemoteExtraParams: config.ocrRemoteExtraParams.trim() || '{}',
    ocrRemoteImageFieldName: config.ocrRemoteImageFieldName.trim() || 'image',
    ocrRemoteRequestFormat: config.ocrRemoteRequestFormat,
    ocrRemoteResultPath: config.ocrRemoteResultPath.trim() || 'data.result'
  })

const loadFromStore = () => {
  const storeConfig = readStoreConfig()
  form.enableLocalOCR = storeConfig.enableLocalOCR
  form.ocrMode = storeConfig.ocrMode
  form.ocrRemoteUrl = storeConfig.ocrRemoteUrl
  form.ocrRemoteExtraParams = storeConfig.ocrRemoteExtraParams
  form.ocrRemoteImageFieldName = storeConfig.ocrRemoteImageFieldName
  form.ocrRemoteRequestFormat = storeConfig.ocrRemoteRequestFormat
  form.ocrRemoteResultPath = storeConfig.ocrRemoteResultPath
  savedSnapshot.value = buildSnapshot(storeConfig)
}

const hasUnsavedChanges = computed(() => buildSnapshot(form) !== savedSnapshot.value)

const extraParamsError = computed(() => {
  const rawText = String(form.ocrRemoteExtraParams || '').trim()
  if (!rawText) {
    return ''
  }

  try {
    const parsed = JSON.parse(rawText)
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return ''
    }
    return '额外参数必须是 JSON 对象，例如 {"lang":"zh"}'
  } catch {
    return '额外参数 JSON 格式不正确'
  }
})

const saveConfig = () => {
  saveMessage.value = ''
  saveError.value = ''

  if (extraParamsError.value) {
    saveError.value = extraParamsError.value
    return
  }

  if (form.ocrMode === 'remote' && form.enableLocalOCR && !form.ocrRemoteUrl.trim()) {
    saveError.value = '启用远程 OCR 时，调用地址不能为空'
    return
  }

  const normalizedConfig: OCRSettingsForm = {
    enableLocalOCR: form.enableLocalOCR,
    ocrMode: form.ocrMode,
    ocrRemoteUrl: form.ocrRemoteUrl.trim(),
    ocrRemoteExtraParams: form.ocrRemoteExtraParams.trim() || '{}',
    ocrRemoteImageFieldName: form.ocrRemoteImageFieldName.trim() || 'image',
    ocrRemoteRequestFormat: form.ocrRemoteRequestFormat,
    ocrRemoteResultPath: form.ocrRemoteResultPath.trim() || 'data.result'
  }

  props.settingsStore.$patch({
    enableLocalOCR: normalizedConfig.enableLocalOCR,
    ocrMode: normalizedConfig.ocrMode,
    ocrRemoteUrl: normalizedConfig.ocrRemoteUrl,
    ocrRemoteExtraParams: normalizedConfig.ocrRemoteExtraParams,
    ocrRemoteImageFieldName: normalizedConfig.ocrRemoteImageFieldName,
    ocrRemoteRequestFormat: normalizedConfig.ocrRemoteRequestFormat,
    ocrRemoteResultPath: normalizedConfig.ocrRemoteResultPath
  })

  form.enableLocalOCR = normalizedConfig.enableLocalOCR
  form.ocrMode = normalizedConfig.ocrMode
  form.ocrRemoteUrl = normalizedConfig.ocrRemoteUrl
  form.ocrRemoteExtraParams = normalizedConfig.ocrRemoteExtraParams
  form.ocrRemoteImageFieldName = normalizedConfig.ocrRemoteImageFieldName
  form.ocrRemoteRequestFormat = normalizedConfig.ocrRemoteRequestFormat
  form.ocrRemoteResultPath = normalizedConfig.ocrRemoteResultPath

  savedSnapshot.value = buildSnapshot(normalizedConfig)
  saveMessage.value = 'OCR 配置已保存并生效'
}

const resetUnsaved = () => {
  saveMessage.value = ''
  saveError.value = ''
  loadFromStore()
}

onMounted(() => {
  loadFromStore()
})

watch(
  () => [
    props.settingsStore.enableLocalOCR,
    props.settingsStore.ocrMode,
    props.settingsStore.ocrRemoteUrl,
    props.settingsStore.ocrRemoteExtraParams,
    props.settingsStore.ocrRemoteImageFieldName,
    props.settingsStore.ocrRemoteRequestFormat,
    props.settingsStore.ocrRemoteResultPath
  ],
  () => {
    if (!hasUnsavedChanges.value) {
      loadFromStore()
    }
  }
)
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">OCR 设置</h3>
      <p class="section-subtitle">配置本地模型或远程 HTTP OCR 接口</p>
    </div>

    <div class="settings-grid">
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">自动 OCR</h4>
          <p class="setting-description">保存图片到历史记录时自动执行 OCR 识别</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input type="checkbox" v-model="form.enableLocalOCR" />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">OCR 方式</h4>
          <p class="setting-description">`local` 使用内置模型，`remote` 使用 HTTP 接口</p>
        </div>
        <div class="setting-control ocr-mode-control">
          <select v-model="form.ocrMode" class="select-input">
            <option value="local">本地模型 (local)</option>
            <option value="remote">远程接口 (remote)</option>
          </select>
        </div>
      </div>

      <template v-if="form.ocrMode === 'remote'">
        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">调用地址</h4>
            <p class="setting-description">远程 OCR API 地址，默认使用 POST 请求</p>
            <input
              v-model.trim="form.ocrRemoteUrl"
              class="text-input"
              type="text"
              placeholder="https://example.com/api/ocr"
            />
          </div>
        </div>

        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">请求格式</h4>
            <p class="setting-description">json: base64 字段；form: 文件字段上传</p>
          </div>
          <div class="setting-control ocr-mode-control">
            <select v-model="form.ocrRemoteRequestFormat" class="select-input">
              <option value="json">JSON</option>
              <option value="form">表单 (multipart/form-data)</option>
            </select>
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">图片字段名称</h4>
            <p class="setting-description">
              例如 `image`，JSON 模式下为 base64 字段，表单模式下为文件字段
            </p>
            <input
              v-model.trim="form.ocrRemoteImageFieldName"
              class="text-input"
              type="text"
              placeholder="image"
            />
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">额外调用参数 (JSON)</h4>
            <p class="setting-description">仅支持 JSON 对象，所有字段会与图片字段一起提交</p>
            <textarea
              v-model="form.ocrRemoteExtraParams"
              class="text-area"
              rows="5"
              placeholder='{"lang":"zh","scene":"general"}'
            ></textarea>
            <p v-if="extraParamsError" class="field-error">{{ extraParamsError }}</p>
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">返回结果读取路径</h4>
            <p class="setting-description">
              仅支持 JSON 响应。示例：`data.result` 对应 `res.data.result`
            </p>
            <input
              v-model.trim="form.ocrRemoteResultPath"
              class="text-input"
              type="text"
              placeholder="data.result"
            />
          </div>
        </div>
      </template>

      <div class="setting-card multi-line-card">
        <div class="setting-info full-width">
          <h4 class="setting-title">保存配置</h4>
          <p class="setting-description">修改后点击保存，配置才会写入并生效</p>
          <div class="save-actions">
            <button class="shortcut-button primary" :disabled="!hasUnsavedChanges" @click="saveConfig">
              保存 OCR 配置
            </button>
            <button class="shortcut-button secondary" :disabled="!hasUnsavedChanges" @click="resetUnsaved">
              放弃修改
            </button>
          </div>
          <p v-if="hasUnsavedChanges" class="unsaved-text">有未保存修改</p>
          <p v-if="saveMessage" class="save-message">{{ saveMessage }}</p>
          <p v-if="saveError" class="field-error">{{ saveError }}</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.multi-line-card {
  align-items: flex-start;
}

.full-width {
  width: 100%;
}

.ocr-mode-control {
  min-width: 220px;
}

.select-input,
.text-input,
.text-area {
  width: 100%;
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  font-size: 14px;
  background: #fff;
  transition: all 0.2s ease;
  box-sizing: border-box;
}

.text-area {
  resize: vertical;
  min-height: 110px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono',
    'Courier New', monospace;
  line-height: 1.4;
}

.select-input:focus,
.text-input:focus,
.text-area:focus {
  outline: none;
  border-color: #007aff;
  box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
}

.field-error {
  margin-top: 8px;
  font-size: 12px;
  color: #d70015;
}

.save-actions {
  display: flex;
  gap: 12px;
  margin-top: 12px;
}

.shortcut-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.unsaved-text {
  margin-top: 10px;
  font-size: 12px;
  color: #b26a00;
}

.save-message {
  margin-top: 10px;
  font-size: 12px;
  color: #1a7f37;
}
</style>
