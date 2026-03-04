import { ClipboardDatabase } from '../../../../infrastructure/database/clipboard-database'
import { ClipboardContent, ClipboardDataConverter, AppInfo, ClipboardHistoryRecord } from '../../types/clipboard-types'
import { ocrService } from '../../../../services/ocr-service'
import logger from '../../../../utils/worker-logger'

export class ClipboardRepository {
    private database: ClipboardDatabase
    private historyCache: Map<string, { id: string; hash: string }> = new Map()
    private idToHashIndex: Map<string, string> = new Map()
    private maxCacheSize: number = 10000
    private lastItemId: string = ''
    private lastItemHash: string = ''

    constructor() {
        this.database = ClipboardDatabase.getInstance()
    }

    public initialize(config?: { maxCacheSize?: number; userDataPath?: string }): void {
        if (config?.maxCacheSize) {
            this.maxCacheSize = config.maxCacheSize
        }
        if (config?.userDataPath) {
            this.database.initialize(config.userDataPath)
        }
        this.initializeHistoryCache()
        this.loadLastItemInfo()
    }

    private initializeHistoryCache(): void {
        try {
            const records = this.database.queryRecordIdsAndHashes({
                limit: this.maxCacheSize,
                orderBy: 'last_used',
                orderDirection: 'DESC'
            })

            this.historyCache.clear()
            this.idToHashIndex.clear()

            for (const record of records) {
                this.historyCache.set(record.hash, {
                    id: record.content_id,
                    hash: record.hash
                })
                this.idToHashIndex.set(record.content_id, record.hash)
            }
        } catch (error) {
            logger.error('[ClipboardRepository] Cache initialization failed:', error)
        }
    }

    private loadLastItemInfo(): void {
        try {
            const lastRecordInfo = this.database.getLastRecordInfo()
            if (lastRecordInfo) {
                this.lastItemId = lastRecordInfo.content_id
                this.lastItemHash = lastRecordInfo.hash
            } else {
                this.lastItemId = ''
                this.lastItemHash = ''
            }
        } catch (error) {
            this.lastItemId = ''
            this.lastItemHash = ''
        }
    }

    public async saveItem(
        content: ClipboardContent,
        contentHash: string,
        options?: {
            appInfo?: AppInfo
            enableLocalOCR?: boolean
        }
    ): Promise<{ saved: boolean; id: string; isNew: boolean }> {
        if (!content || ClipboardDataConverter.isEmptyContent(content)) {
            return { saved: false, id: '', isNew: false }
        }

        const now = Date.now()

        // Check if it's the same as the last item
        if (this.lastItemHash === contentHash) {
            this.database.updateLastUsed(this.lastItemId, now)
            return { saved: true, id: this.lastItemId, isNew: false }
        }

        // Check cache
        const cachedRecord = this.historyCache.get(contentHash)
        if (cachedRecord) {
            this.database.updateLastUsed(cachedRecord.id, now)
            this.lastItemId = cachedRecord.id
            this.lastItemHash = contentHash
            return { saved: true, id: cachedRecord.id, isNew: false }
        }

        // Verify against database (double check if cache missed but DB has it?)
        // The queryRecordIdsAndHashes loads most recent, but maybe it's an old item not in cache.
        // For now, let's assume if it's not in cache (and cache is full-ish), we treat as new or rely on DB constraints?
        // The original worker logic: check cache -> if not, insert new.
        // Assuming we want to preserve that behavior for now.

        const newItemId = ClipboardDataConverter.generateId()
        const availableFormats = ClipboardDataConverter.getAvailableFormats(content)

        let sourceAppId: string | undefined = undefined
        if (options?.appInfo) {
            sourceAppId = this.database.insertOrUpdateApp(
                options.appInfo.application,
                options.appInfo.path,
                options.appInfo.icon
            ) || undefined
        }

        this.database.insertRecord(
            newItemId,
            content,
            ClipboardDataConverter.detectPrimaryType(content, availableFormats),
            availableFormats,
            now,
            now,
            sourceAppId,
            contentHash
        )

        this.addToCache(newItemId, contentHash)
        this.lastItemId = newItemId
        this.lastItemHash = contentHash

        // Async OCR
        if (options?.enableLocalOCR && content.image) {
            this.performAsyncOCR(newItemId, content.image)
        }

        return { saved: true, id: newItemId, isNew: true }
    }

    private addToCache(id: string, hash: string): void {
        if (this.historyCache.size >= this.maxCacheSize) {
            const firstKey = this.historyCache.keys().next().value
            if (firstKey) {
                const oldRecord = this.historyCache.get(firstKey)
                if (oldRecord) {
                    this.idToHashIndex.delete(oldRecord.id)
                }
                this.historyCache.delete(firstKey)
            }
        }
        this.historyCache.set(hash, { id, hash })
        this.idToHashIndex.set(id, hash)
    }

    public getById(id: string): ClipboardHistoryRecord | undefined {
        return this.database.getRecordByContentId(id) || undefined
    }

    public getLastItemHash(): string {
        return this.lastItemHash
    }

    public syncDeletedRecords(deletedIds: string[]): string[] {
        if (deletedIds.includes('*')) {
            this.lastItemId = ''
            this.lastItemHash = ''
            this.historyCache.clear()
            this.idToHashIndex.clear()
            return [] // All deleted
        }

        const deletedHashes: string[] = []
        for (const [hash, record] of this.historyCache.entries()) {
            if (deletedIds.includes(record.id)) {
                deletedHashes.push(hash)
            }
        }

        for (const hash of deletedHashes) {
            this.removeFromCache(hash)
        }

        if (deletedIds.includes(this.lastItemId)) {
            this.loadLastItemInfo()
        }

        return deletedHashes
    }

    public syncDeletedRecordsByHash(deletedHashes: string[]): void {
        if (deletedHashes.includes(this.lastItemHash)) {
            this.lastItemId = ''
            this.lastItemHash = ''
            this.loadLastItemInfo()
        }
        for (const hash of deletedHashes) {
            this.removeFromCache(hash)
        }
    }

    public syncDeletedRecordsByIds(deletedIds: string[]): void {
        if (deletedIds.includes(this.lastItemId)) {
            this.lastItemId = ''
            this.lastItemHash = ''
            this.loadLastItemInfo()
        }
        for (const id of deletedIds) {
            const hash = this.idToHashIndex.get(id)
            if (hash) {
                this.historyCache.delete(hash)
                this.idToHashIndex.delete(id)
            }
        }
    }

    private removeFromCache(hash: string): void {
        const record = this.historyCache.get(hash)
        if (record) {
            this.idToHashIndex.delete(record.id)
        }
        this.historyCache.delete(hash)
    }

    // Moved from worker
    private async performAsyncOCR(contentId: string, imageBuffer: Buffer): Promise<void> {
        try {
            if (!ocrService.ready) {
                // OCR service should be initialized from worker or elsewhere
                // For now we assume the worker initializes OCR service before calling repo (or we check status)
                // But repo doesn't have config.
            }
            // ... (Logic relies on ocrService singleton, so it's fine)
            const ocrResult = await ocrService.recognizeText(imageBuffer)
            if (ocrResult.success && ocrResult.text && ocrResult.text.trim()) {
                this.database.updateOCR(contentId, ocrResult.text)
                // Need to notify frontend? Repo shouldn't emit events directly maybe?
                // Return result or let worker listen to change?
                // Async nature makes it hard to return. 
                // Maybe we just do DB update here, and Worker can have a callback?
            }
        } catch (error) {
            logger.error('[ClipboardRepository] OCR failed:', error)
        }
    }

    // Helper to re-expose database methods needed
    public updateOCR(id: string, text: string): boolean {
        return this.database.updateOCR(id, text)
    }
}
