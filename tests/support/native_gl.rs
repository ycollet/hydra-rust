use std::ffi::{CString, c_void};
use std::sync::Arc;

#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLChoosePixelFormat(
        attributes: *const i32,
        format: *mut *mut c_void,
        count: *mut i32,
    ) -> i32;
    fn CGLDestroyPixelFormat(format: *mut c_void) -> i32;
    fn CGLCreateContext(format: *mut c_void, share: *mut c_void, context: *mut *mut c_void) -> i32;
    fn CGLSetCurrentContext(context: *mut c_void) -> i32;
    fn CGLGetCurrentContext() -> *mut c_void;
    fn CGLDestroyContext(context: *mut c_void) -> i32;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
}

pub struct NativeContext {
    context: *mut c_void,
    previous: *mut c_void,
}

impl NativeContext {
    pub fn new() -> Result<Self, String> {
        let attributes = [99, 0x3200, 0];
        let mut format = std::ptr::null_mut();
        let mut count = 0;
        let mut context = std::ptr::null_mut();
        unsafe {
            let error = CGLChoosePixelFormat(attributes.as_ptr(), &mut format, &mut count);
            if error != 0 || format.is_null() {
                return Err(format!("OpenGL pixel format selection failed: {error}"));
            }
            let error = CGLCreateContext(format, std::ptr::null_mut(), &mut context);
            CGLDestroyPixelFormat(format);
            if error != 0 || context.is_null() {
                return Err(format!("OpenGL context creation failed: {error}"));
            }
            let previous = CGLGetCurrentContext();
            let error = CGLSetCurrentContext(context);
            if error != 0 {
                CGLDestroyContext(context);
                return Err(format!("OpenGL context activation failed: {error}"));
            }
            Ok(Self { context, previous })
        }
    }

    pub fn gl_with_loader(
        &self,
        override_symbol: impl Fn(&str) -> Option<*const c_void>,
    ) -> Arc<glow::Context> {
        Arc::new(unsafe {
            glow::Context::from_loader_function(|symbol| {
                override_symbol(symbol).unwrap_or_else(|| {
                    let name = CString::new(symbol).unwrap();
                    dlsym((-2_isize) as *mut c_void, name.as_ptr()).cast_const()
                })
            })
        })
    }
}

impl Drop for NativeContext {
    fn drop(&mut self) {
        unsafe {
            CGLSetCurrentContext(self.previous);
            CGLDestroyContext(self.context);
        }
    }
}
