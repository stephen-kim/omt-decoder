use libvmx_sys::root::*;
use ::std::ptr;

pub struct VmxDecoder {
    instance: *mut VMX_INSTANCE,
    width: u32,
    height: u32,
    /// Pre-allocated decode buffer, reused every frame to avoid allocation overhead.
    buffer: Vec<u8>,
    /// Mutable copy of compressed input — VMX_LoadFrom may write to its input buffer.
    input_buf: Vec<u8>,
}

unsafe impl Send for VmxDecoder {}

impl VmxDecoder {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let size = VMX_SIZE {
            width: width as i32,
            height: height as i32,
        };
        // Decoder must use DEFAULT profile — the compressed stream carries
        // its own quality info. Using an encoder profile here causes internal
        // buffer size mismatch and crashes (SEGV).
        let instance = unsafe {
            VMX_Create(
                size,
                VMX_PROFILE_VMX_PROFILE_DEFAULT,
                VMX_COLORSPACE_VMX_COLORSPACE_UNDEFINED,
            )
        };
        if instance.is_null() {
            return None;
        }
        // VMX codec works in 16-row slices. If height is not a multiple of 16,
        // the decoder writes up to the next multiple. Allocate enough to avoid overflow.
        let aligned_height = (height + 15) & !15;
        let buf_size = (width * aligned_height * 4) as usize;
        Some(VmxDecoder {
            instance,
            width,
            height,
            buffer: vec![0u8; buf_size],
            input_buf: Vec::with_capacity(1024 * 1024),
        })
    }

    /// Decode a VMX1 compressed frame into BGRA pixels.
    /// Returns a slice of the internal buffer (zero-copy, valid until next decode call).
    pub fn decode(&mut self, compressed: &[u8]) -> Option<&[u8]> {
        if compressed.is_empty() {
            return None;
        }
        let stride = (self.width * 4) as i32;

        // Copy to mutable buffer — VMX_LoadFrom's signature takes non-const BYTE*
        // and may modify the input during parsing.
        self.input_buf.clear();
        self.input_buf.extend_from_slice(compressed);

        unsafe {
            let hr = VMX_LoadFrom(
                self.instance,
                self.input_buf.as_mut_ptr(),
                self.input_buf.len() as i32,
            );
            if hr != VMX_ERR_VMX_ERR_OK {
                return None;
            }

            let hr = VMX_DecodeBGRA(self.instance, self.buffer.as_mut_ptr(), stride);
            if hr != VMX_ERR_VMX_ERR_OK {
                return None;
            }
        }

        Some(&self.buffer)
    }
}

impl Drop for VmxDecoder {
    fn drop(&mut self) {
        if !self.instance.is_null() {
            unsafe {
                VMX_Destroy(self.instance);
            }
            self.instance = ptr::null_mut();
        }
    }
}
