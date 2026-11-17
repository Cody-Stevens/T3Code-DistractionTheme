//! Synchronous libavif boundary. All unsafe operations are confined to this module.
#![allow(unsafe_code)]
use super::{AvifEncoder, AvifOptions, EncodeSpeed, validate};
use dt_core::{Error, Frame, Result};
use std::sync::Arc;
unsafe extern "C" {
    fn dt_render_encode(
        frames: *const *const u8,
        count: u32,
        width: u32,
        height: u32,
        quality: u32,
        speed: u32,
        threads: u32,
        durations: *const u32,
        plays: u32,
        output: *mut *mut u8,
        length: *mut usize,
    ) -> i32;
    fn dt_render_free(output: *mut u8);
}
/// In-process libavif/libaom, built for the generic CPU with no NASM requirement.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeAvifEncoder;
impl AvifEncoder for NativeAvifEncoder {
    fn encode(&self, frames: &[Arc<Frame>], options: &AvifOptions) -> Result<Vec<u8>> {
        validate(frames, options)?;
        if frames.len() > u32::MAX as usize || options.plays > i32::MAX as u32 {
            return Err(Error::InvalidInput("AVIF sequence count overflow"));
        }
        let pointers: Vec<_> = frames.iter().map(|f| f.pixels().as_ptr()).collect();
        let mut output = std::ptr::null_mut();
        let mut length = 0;
        // SAFETY: Frame validates all RGB lengths; validate checks common dimensions, nonzero count,
        // duration count, quality and threads. All borrowed buffers outlive this synchronous call.
        // C allocates output only on success; it returns ownership for copying, then dt_render_free.
        let code = unsafe {
            dt_render_encode(
                pointers.as_ptr(),
                frames.len() as u32,
                frames[0].width(),
                frames[0].height(),
                u32::from(options.quality),
                match options.speed {
                    EncodeSpeed::Fast => 10,
                    EncodeSpeed::Favicon => 8,
                    EncodeSpeed::Ahead => 6,
                },
                options.threads as u32,
                options.durations_ms.as_ptr(),
                options.plays,
                &mut output,
                &mut length,
            )
        };
        if code != 0 {
            return Err(Error::Protocol("native AVIF encoding failed"));
        }
        if output.is_null() {
            return Err(Error::Protocol("native AVIF returned no bytes"));
        }
        // SAFETY: successful C call returns length initialized bytes allocated by malloc.
        let data = unsafe { std::slice::from_raw_parts(output, length) }.to_vec();
        // SAFETY: free exactly the owned pointer returned by the same C library, once.
        unsafe { dt_render_free(output) };
        Ok(data)
    }
}
