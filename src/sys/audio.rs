//! Which processes have sound going right now, playing or recording, from the Windows audio
//! session list (the one behind the Volume mixer). No stream is opened and no process is touched.

use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const ENUMERATOR: Guid = Guid(0xBCDE0395, 0xE52F, 0x467C, [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E]);
const IID_ENUMERATOR: Guid = Guid(0xA95664D2, 0x9614, 0x4F35, [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6]);
const IID_MANAGER: Guid = Guid(0x77AA99A0, 0x1BD6, 0x484F, [0x8B, 0xC7, 0x2C, 0x65, 0x4C, 0x9A, 0x9B, 0x6F]);
const IID_CONTROL2: Guid = Guid(0xBFB7FF88, 0x7239, 0x4FC9, [0x8F, 0xA2, 0x07, 0xC9, 0x50, 0xBE, 0x9C, 0x6D]);

const CLSCTX_ALL: u32 = 23;
const ALL_FLOWS: i32 = 2;
const DEVICE_ACTIVE: u32 = 1;
const SESSION_ACTIVE: i32 = 1;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const Guid, outer: *const c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
}

/// A COM interface pointer, released on drop
struct Com(*mut c_void);

impl Com {
    fn new(hr: i32, p: *mut c_void) -> Option<Com> {
        (hr >= 0 && !p.is_null()).then_some(Com(p))
    }

    /// The vtable's entry at `slot`
    ///
    /// # Safety
    /// `F` has to be that method's signature.
    unsafe fn f<F: Copy>(&self, slot: usize) -> F {
        let table = *(self.0 as *const *const *const c_void);
        std::mem::transmute_copy(&*table.add(slot))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = self.f(2);
            release(self.0);
        }
    }
}

/// None when Windows couldn't say: then every app has to be treated as if it had sound
pub fn active() -> Option<HashSet<u32>> {
    unsafe {
        let hr = CoInitializeEx(null_mut(), 0);
        let found = scan();
        if hr >= 0 {
            CoUninitialize();
        }
        found
    }
}

unsafe fn scan() -> Option<HashSet<u32>> {
    type Out = *mut *mut c_void;
    let mut p = null_mut();
    let hr = CoCreateInstance(&ENUMERATOR, null(), CLSCTX_ALL, &IID_ENUMERATOR, &mut p);
    let devices = Com::new(hr, p)?;

    let mut p = null_mut();
    let list: unsafe extern "system" fn(*mut c_void, i32, u32, Out) -> i32 = devices.f(3);
    let endpoints = Com::new(list(devices.0, ALL_FLOWS, DEVICE_ACTIVE, &mut p), p)?;

    let mut n = 0u32;
    let count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = endpoints.f(3);
    if count(endpoints.0, &mut n) < 0 {
        return None;
    }

    let mut found = HashSet::new();
    for i in 0..n {
        let mut p = null_mut();
        let item: unsafe extern "system" fn(*mut c_void, u32, Out) -> i32 = endpoints.f(4);
        let device = Com::new(item(endpoints.0, i, &mut p), p)?;

        let mut p = null_mut();
        let activate: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void, Out) -> i32 = device.f(3);
        let manager = Com::new(activate(device.0, &IID_MANAGER, CLSCTX_ALL, null(), &mut p), p)?;

        let mut p = null_mut();
        let sessions: unsafe extern "system" fn(*mut c_void, Out) -> i32 = manager.f(5);
        let sessions = Com::new(sessions(manager.0, &mut p), p)?;

        let mut m = 0i32;
        let count: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = sessions.f(3);
        if count(sessions.0, &mut m) < 0 {
            return None;
        }
        for j in 0..m {
            let mut p = null_mut();
            let get: unsafe extern "system" fn(*mut c_void, i32, Out) -> i32 = sessions.f(4);
            let control = Com::new(get(sessions.0, j, &mut p), p)?;

            let mut state = 0i32;
            let get_state: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = control.f(3);
            if get_state(control.0, &mut state) < 0 {
                return None;
            }
            if state != SESSION_ACTIVE {
                continue;
            }

            let mut p = null_mut();
            let query: unsafe extern "system" fn(*mut c_void, *const Guid, Out) -> i32 = control.f(0);
            let control2 = Com::new(query(control.0, &IID_CONTROL2, &mut p), p)?;

            let mut pid = 0u32;
            let get_pid: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = control2.f(14);
            if get_pid(control2.0, &mut pid) < 0 {
                return None;
            }
            if pid != 0 {
                found.insert(pid);
            }
        }
    }
    Some(found)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_sessions() {
        let t = std::time::Instant::now();
        let found = super::active();
        println!("{:?} processes with sound, in {:?}", found.as_ref().map(|f| f.len()), t.elapsed());
        assert!(found.is_some());
    }
}
