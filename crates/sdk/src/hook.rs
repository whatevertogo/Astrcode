pub trait PolicyHook: Send + Sync {
    fn before_invoke(&self, capability: &str) -> bool;
}
