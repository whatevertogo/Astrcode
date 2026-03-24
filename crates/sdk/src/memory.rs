pub trait MemoryProvider: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
}
