//! The one kernel-boundary implementation in this crate: the DRM device-query ioctl.
#![allow(unsafe_code)]

use rustix::ioctl::{self, Ioctl, IoctlOutput, Opcode};
use std::os::fd::AsFd;

const DRM_IOCTL_XE_DEVICE_QUERY: u32 = 0xC028_6440; // _IOWR('d', 0x40, 40)

struct DeviceQuery<'a>(&'a mut [u8; 40]);

unsafe impl Ioctl for DeviceQuery<'_> {
    type Output = ();
    const IS_MUTATING: bool = true;
    fn opcode(&self) -> Opcode {
        DRM_IOCTL_XE_DEVICE_QUERY as Opcode
    }
    fn as_ptr(&mut self) -> *mut core::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
    unsafe fn output_from_ptr(_: IoctlOutput, _: *mut core::ffi::c_void) -> rustix::io::Result<()> {
        Ok(())
    }
}

/// Two-call query protocol on `drm_xe_device_query` (40 bytes, native endian: extensions u64,
/// query u32, size u32, data u64). The first call reports the buffer size, the second fills it;
/// `prefill` is written into that buffer before the second call (UC_FW_VERSION asks by `uc_type`).
pub fn query(fd: &impl AsFd, query: u32, prefill: &[u8]) -> rustix::io::Result<Vec<u8>> {
    let mut arg = [0u8; 40];
    arg[8..12].copy_from_slice(&query.to_ne_bytes());
    // SAFETY: arg is a valid, writable drm_xe_device_query; the kernel writes only `size` on the
    // first call.
    unsafe { ioctl::ioctl(fd, DeviceQuery(&mut arg))? };
    let size = u32::from_ne_bytes(arg[12..16].try_into().unwrap()) as usize;
    let mut buf = vec![0u8; size];
    buf[..prefill.len().min(size)].copy_from_slice(&prefill[..prefill.len().min(size)]);
    arg[16..24].copy_from_slice(&(buf.as_mut_ptr() as u64).to_ne_bytes());
    // SAFETY: buf outlives the call and has exactly `size` bytes, which is what the kernel writes.
    unsafe { ioctl::ioctl(fd, DeviceQuery(&mut arg))? };
    Ok(buf)
}
