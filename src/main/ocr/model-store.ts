import { createHash } from 'node:crypto'
import { createReadStream, createWriteStream, promises as fs } from 'node:fs'
import path from 'node:path'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { open, type ZipFile, type Entry } from 'yauzl'
import {
  OCR_MODELS,
  getOCRModel,
  getOCRModelRevision,
  getOCRModelURL,
  type OCRModel,
  type OCRModelId,
  type OCRModelStatus
} from '../../shared/ocr-models'

const MODEL_FILES = { det: 'det.onnx', rec: 'rec.onnx', dictionary: 'dict.txt' } as const
type ModelFile = keyof typeof MODEL_FILES
interface FileIntegrity {
  size: number
  sha256: string
}
interface ModelManifest {
  format: 1
  modelId: OCRModelId
  revision: string
  files: Record<ModelFile, FileIntegrity>
}

export interface InstalledOCRModel {
  model: OCRModel
  directory: string
  det: string
  rec: string
  dictionary: string
}

async function hashFile(file: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest('hex')
}

export async function getInstalledOCRModel(
  root: string,
  id: OCRModelId,
  verify = false
): Promise<InstalledOCRModel | null> {
  const model = getOCRModel(id)
  const directory = path.join(root, model.id)
  try {
    const manifest = JSON.parse(
      await fs.readFile(path.join(directory, 'manifest.json'), 'utf8')
    ) as Partial<ModelManifest> | null
    if (
      !manifest ||
      manifest.format !== 1 ||
      manifest.modelId !== model.id ||
      manifest.revision !== getOCRModelRevision(model)
    )
      return null
    for (const [role, name] of Object.entries(MODEL_FILES)) {
      const entry = manifest.files?.[role as ModelFile]
      if (
        !entry ||
        typeof entry.sha256 !== 'string' ||
        !Number.isSafeInteger(entry.size) ||
        entry.size <= 0 ||
        !/^[a-f0-9]{64}$/.test(entry.sha256)
      )
        return null
      const file = path.join(directory, name)
      const stat = await fs.stat(file)
      if (!stat.isFile() || stat.size !== entry.size) return null
      if (verify && (await hashFile(file)) !== entry.sha256) return null
    }
    return {
      model,
      directory,
      det: path.join(directory, MODEL_FILES.det),
      rec: path.join(directory, MODEL_FILES.rec),
      dictionary: path.join(directory, MODEL_FILES.dictionary)
    }
  } catch (error) {
    if (error instanceof SyntaxError || (error as NodeJS.ErrnoException).code === 'ENOENT')
      return null
    throw error
  }
}

interface ModelStoreOptions {
  root: string
  fetch: (input: string, init?: RequestInit) => Promise<Response>
  onStatus?: (status: OCRModelStatus) => void
}

export class OCRModelStore {
  private tasks = new Map<
    OCRModelId,
    { controller: AbortController; promise: Promise<InstalledOCRModel> }
  >()
  private statuses = new Map<OCRModelId, OCRModelStatus>()

  constructor(private readonly options: ModelStoreOptions) {}

  public async list(): Promise<OCRModelStatus[]> {
    return Promise.all(
      OCR_MODELS.map(async (model): Promise<OCRModelStatus> => {
        const status = this.statuses.get(model.id)
        if (status?.state === 'downloading') return status
        const installed = await getInstalledOCRModel(this.options.root, model.id)
        if (installed)
          return { id: model.id, state: 'ready', receivedBytes: model.size, totalBytes: model.size }
        return status?.state === 'error'
          ? status
          : { id: model.id, state: 'not-downloaded', receivedBytes: 0, totalBytes: model.size }
      })
    )
  }

  public download(id: unknown): Promise<InstalledOCRModel> {
    const model = getOCRModel(id)
    const task = this.tasks.get(model.id)
    if (task) return task.promise
    const controller = new AbortController()
    const promise = this.install(model, controller.signal).finally(() =>
      this.tasks.delete(model.id)
    )
    this.tasks.set(model.id, { controller, promise })
    return promise
  }

  public cancel(id: unknown): void {
    const model = getOCRModel(id)
    this.tasks.get(model.id)?.controller.abort(new Error('模型下载已取消'))
  }

  public cancelAll(): void {
    for (const task of this.tasks.values()) task.controller.abort(new Error('应用正在退出'))
  }

  private publish(status: OCRModelStatus): void {
    this.statuses.set(status.id, status)
    this.options.onStatus?.({ ...status })
  }

  private async install(model: OCRModel, cancelSignal: AbortSignal): Promise<InstalledOCRModel> {
    let staging: string | undefined
    const signal = AbortSignal.any([cancelSignal, AbortSignal.timeout(15 * 60_000)])
    const status: OCRModelStatus = {
      id: model.id,
      state: 'downloading',
      stage: 'verifying',
      receivedBytes: 0,
      totalBytes: model.size
    }
    this.publish(status)
    try {
      const installed = await getInstalledOCRModel(this.options.root, model.id, true)
      signal.throwIfAborted()
      if (installed) {
        this.publish({ ...status, state: 'ready', stage: undefined, receivedBytes: model.size })
        return installed
      }
      signal.throwIfAborted()
      await fs.mkdir(this.options.root, { recursive: true })
      staging = await fs.mkdtemp(path.join(this.options.root, '.download-' + model.id + '-'))
      const archive = path.join(staging, 'model.zip.part')
      status.stage = 'downloading'
      this.publish(status)
      const response = await this.options.fetch(getOCRModelURL(model), {
        signal,
        cache: 'no-store'
      })
      if (!response.ok || !response.body) throw new Error('模型下载失败，HTTP ' + response.status)
      const hash = createHash('sha256')
      let lastProgress = 0
      const progress = new Transform({
        transform: (chunk: Buffer, _encoding, callback): void => {
          status.receivedBytes += chunk.length
          if (status.receivedBytes > model.size) {
            callback(new Error('模型下载大小与发布信息不符'))
            return
          }
          hash.update(chunk)
          if (Date.now() - lastProgress >= 100) {
            lastProgress = Date.now()
            this.publish(status)
          }
          callback(null, chunk)
        }
      })
      await pipeline(
        Readable.fromWeb(response.body as import('node:stream/web').ReadableStream<Uint8Array>),
        progress,
        createWriteStream(archive, { flags: 'wx' }),
        { signal }
      )
      if (
        status.receivedBytes !== model.size ||
        (model.sha256 && hash.digest('hex') !== model.sha256)
      )
        throw new Error('模型下载校验失败，请重试')
      status.stage = 'extracting'
      this.publish(status)
      const files = await this.extract(archive, staging, signal)
      if (!files.det) {
        if (model.id === 'ppocr-v4') throw new Error('下载包中缺少文字检测模型')
        status.stage = 'preparing-detector'
        this.publish(status)
        // 部分语言包仅含识别模型，上游要求搭配中英文检测模型。
        const detector = await this.waitForDetector(signal)
        signal.throwIfAborted()
        const target = path.join(staging, MODEL_FILES.det)
        await fs.copyFile(detector.det, target)
        files.det = { size: (await fs.stat(target)).size, sha256: await hashFile(target) }
      }
      if (!files.rec || !files.dictionary) throw new Error('下载包中缺少识别模型或字典')
      status.stage = 'verifying'
      this.publish(status)
      const dictionary = await fs.readFile(path.join(staging, MODEL_FILES.dictionary), 'utf8')
      if (!dictionary.trim()) throw new Error('OCR 字典为空')
      const manifest: ModelManifest = {
        format: 1,
        modelId: model.id,
        revision: getOCRModelRevision(model),
        files: { det: files.det, rec: files.rec, dictionary: files.dictionary }
      }
      await fs.writeFile(path.join(staging, 'manifest.json'), JSON.stringify(manifest), {
        flag: 'wx'
      })
      await fs.unlink(archive)
      signal.throwIfAborted()
      const directory = path.join(this.options.root, model.id)
      await fs.rm(directory, { recursive: true, force: true })
      await fs.rename(staging, directory)
      staging = undefined
      this.publish({ ...status, state: 'ready', stage: undefined })
      return {
        model,
        directory,
        det: path.join(directory, MODEL_FILES.det),
        rec: path.join(directory, MODEL_FILES.rec),
        dictionary: path.join(directory, MODEL_FILES.dictionary)
      }
    } catch (error) {
      const reason = signal.aborted ? signal.reason : error
      const message = reason instanceof Error ? reason.message : String(reason)
      this.publish({ ...status, state: 'error', stage: undefined, error: message })
      throw new Error(message, { cause: error })
    } finally {
      if (staging) await fs.rm(staging, { recursive: true, force: true })
    }
  }

  private async waitForDetector(signal: AbortSignal): Promise<InstalledOCRModel> {
    signal.throwIfAborted()
    let abort: (() => void) | undefined
    const cancelled = new Promise<never>((_, reject) => {
      abort = () => reject(signal.reason)
      signal.addEventListener('abort', abort, { once: true })
    })
    try {
      // 检测模型可能被其他语言包共用，取消等待不终止共享下载。
      return await Promise.race([this.download('ppocr-v4'), cancelled])
    } finally {
      if (abort) signal.removeEventListener('abort', abort)
    }
  }

  private async extract(
    archive: string,
    directory: string,
    signal: AbortSignal
  ): Promise<Partial<Record<ModelFile, FileIntegrity>>> {
    const zip = await new Promise<ZipFile>((resolve, reject) => {
      open(archive, { lazyEntries: true, validateEntrySizes: true }, (error, result) =>
        error || !result ? reject(error || new Error('无法读取模型压缩包')) : resolve(result)
      )
    })
    const files: Partial<Record<ModelFile, FileIntegrity>> = {}
    let transfer: Promise<void> | undefined
    try {
      await new Promise<void>((resolve, reject) => {
        const fail = (error: Error): void => {
          zip.close()
          reject(error)
        }
        const abort = (): void => fail(signal.reason)
        signal.addEventListener('abort', abort, { once: true })
        zip.once('close', () => signal.removeEventListener('abort', abort))
        zip.once('error', fail)
        zip.once('end', resolve)
        zip.on('entry', (entry: Entry) => {
          const name = path.posix.basename(entry.fileName)
          const role: ModelFile | undefined =
            /(?:^|_)det(?:_|\.)[^/]*\.onnx$/i.test(name) || /(?:^|_)det\.onnx$/i.test(name)
              ? 'det'
              : /(?:^|_)rec(?:_|\.)[^/]*\.onnx$/i.test(name) || /(?:^|_)rec\.onnx$/i.test(name)
                ? 'rec'
                : /\.txt$/i.test(name) && !/readme|license/i.test(name)
                  ? 'dictionary'
                  : undefined
          if (!role) {
            zip.readEntry()
            return
          }
          if (
            files[role] ||
            entry.uncompressedSize <= 0 ||
            entry.uncompressedSize > 512 * 1024 * 1024 ||
            ((entry.externalFileAttributes >>> 16) & 0xf000) === 0xa000
          ) {
            fail(new Error('模型压缩包包含重复或无效的文件'))
            return
          }
          zip.openReadStream(entry, (error, stream) => {
            if (error || !stream) {
              fail(error || new Error('无法解压模型文件'))
              return
            }
            const hash = createHash('sha256')
            const hashing = new Transform({
              transform(chunk: Buffer, _encoding, callback): void {
                hash.update(chunk)
                callback(null, chunk)
              }
            })
            // 输出文件名固定，不使用压缩包内的路径。
            transfer = pipeline(
              stream,
              hashing,
              createWriteStream(path.join(directory, MODEL_FILES[role]), { flags: 'wx' }),
              { signal }
            )
            transfer.then(() => {
              files[role] = { size: entry.uncompressedSize, sha256: hash.digest('hex') }
              zip.readEntry()
            }, fail)
          })
        })
        if (signal.aborted) abort()
        else zip.readEntry()
      })
    } finally {
      zip.close()
      // 确保流关闭后再清理临时目录，兼容 Windows 的文件占用规则。
      if (transfer)
        await transfer.then(
          () => undefined,
          () => undefined
        )
    }
    return files
  }
}
