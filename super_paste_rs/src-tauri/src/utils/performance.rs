use std::time::Instant;

pub struct PerformanceMonitor {
    start: Instant,
    name: String,
}

impl PerformanceMonitor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            start: Instant::now(),
            name: name.into(),
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    pub fn log(&self) {
        let duration = self.elapsed();
        tracing::debug!("[性能] {} 耗时: {:?}", self.name, duration);

        if duration.as_millis() > 100 {
            tracing::warn!("[性能] {} 耗时过长: {:?}", self.name, duration);
        }
    }
}

impl Drop for PerformanceMonitor {
    fn drop(&mut self) {
        self.log();
    }
}
