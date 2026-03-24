use libvmx_sys::root::*;
use ::std::ptr;

pub struct VmxDecoder {
    instance: *mut VMX_INSTANCE,
    width: u32,
    height: u32,
}

unsafe impl Send for VmxDecoder {}

impl VmxDecoder {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let size = VMX_SIZE {
            width: width as i32,
            height: height as i32,
        };
        let instance = unsafe {
            VMX_Create(
                size,
                VMX_PROFILE_VMX_PROFILE_OMT_HQ,
                VMX_COLORSPACE_VMX_COLORSPACE_UNDEFINED,
            )
        };
        if instance.is_null() {
            return None;
        }
        Some(VmxDecoder {
            instance,
            width,
            height,
        })
    }

    /// Decode a VMX1 compressed frame into BGRA pixels.
    /// Returns the BGRA buffer on success.
    pub fn decode(&mut self, compressed: &[u8]) -> Option<Vec<u8>> {
        let stride = (self.width * 4) as i32;
        let buf_size = (stride as u32 * self.height) as usize;
        let mut dst = vec![0u8; buf_size];

        unsafe {
            let hr = VMX_LoadFrom(
                self.instance,
                compressed.as_ptr() as *mut u8,
                compressed.len() as i32,
            );
            if hr != VMX_ERR_VMX_ERR_OK {
                return None;
            }

            let hr = VMX_DecodeBGRA(self.instance, dst.as_mut_ptr(), stride);
            if hr != VMX_ERR_VMX_ERR_OK {
                return None;
            }
        }

        Some(dst)
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

/// Stateless convenience function. Creates a decoder, decodes one frame, and discards the decoder.
/// For sustained playback, use VmxDecoder directly to avoid re-allocating each frame.
pub fn decode_frame(compressed: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    // NOTE: In the main loop we should keep VmxDecoder alive and reuse it.
    // This function exists for simplicity; the main loop should use VmxDecoder directly.
    let mut dec = VmxDecoder::new(width, height)?;
    dec.decode(compressed)
}
