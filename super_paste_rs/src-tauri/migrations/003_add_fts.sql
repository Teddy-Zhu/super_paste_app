-- 全文搜索表
CREATE VIRTUAL TABLE clipboard_fts USING fts5(
    content_id UNINDEXED,
    text_content,
    html_content,
    ocr_text,
    tokenize='trigram'
);

-- 触发器：插入时同步到 FTS
CREATE TRIGGER clipboard_fts_insert AFTER INSERT ON clipboard_content
BEGIN
    INSERT INTO clipboard_fts(content_id, text_content, html_content, ocr_text)
    VALUES (
        NEW.content_id,
        NEW.text_content,
        NEW.html_content,
        (SELECT text_content FROM ocr_results WHERE content_id = NEW.content_id)
    );
END;

-- 触发器：更新时同步到 FTS
CREATE TRIGGER clipboard_fts_update AFTER UPDATE ON clipboard_content
BEGIN
    UPDATE clipboard_fts
    SET text_content = NEW.text_content,
        html_content = NEW.html_content
    WHERE content_id = NEW.content_id;
END;

-- 触发器：删除时同步到 FTS
CREATE TRIGGER clipboard_fts_delete AFTER DELETE ON clipboard_content
BEGIN
    DELETE FROM clipboard_fts WHERE content_id = OLD.content_id;
END;

-- 触发器：OCR 完成时更新 FTS
CREATE TRIGGER ocr_fts_insert AFTER INSERT ON ocr_results
BEGIN
    UPDATE clipboard_fts
    SET ocr_text = NEW.text_content
    WHERE content_id = NEW.content_id;
END;
