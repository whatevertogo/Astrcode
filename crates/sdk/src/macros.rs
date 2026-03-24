#[macro_export]
macro_rules! declare_tool {
    ($handler:ty) => {
        #[no_mangle]
        pub extern "C" fn _astrcode_plugin_entry() -> *mut std::ffi::c_void {
            let handler = <$handler>::default();
            Box::into_raw(Box::new(handler)) as *mut std::ffi::c_void
        }
    };
}
