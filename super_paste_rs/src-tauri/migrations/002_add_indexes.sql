-- 性能优化索引
CREATE INDEX idx_items_created ON clipboard_items(created_at DESC);
CREATE INDEX idx_items_last_used ON clipboard_items(last_used_at DESC);
CREATE INDEX idx_items_type ON clipboard_items(primary_type, created_at DESC);
CREATE INDEX idx_items_pinned ON clipboard_items(is_pinned) WHERE is_pinned = 1;
CREATE INDEX idx_items_favorite ON clipboard_items(is_favorite) WHERE is_favorite = 1;
CREATE INDEX idx_items_hash ON clipboard_items(content_hash);
CREATE INDEX idx_apps_bundle ON apps(bundle_id);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
