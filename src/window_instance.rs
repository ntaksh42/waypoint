//! HWND が再利用されても、別名を付けた窓と新しい窓を区別するための識別子。

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{GetPropW, IsWindow, RemovePropW, SetPropW};
use windows::core::{GUID, Result, w};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowInstance {
    pub hwnd: isize,
    pub cookie: usize,
}

impl WindowInstance {
    pub fn read(hwnd: isize) -> Option<Self> {
        if hwnd == 0 {
            return None;
        }
        let value = unsafe { GetPropW(HWND(hwnd as *mut _), w!("Waypoint.Instance")) };
        (!value.0.is_null()).then_some(Self {
            hwnd,
            cookie: value.0 as usize,
        })
    }

    pub fn is_current(self) -> bool {
        Self::read(self.hwnd) == Some(self)
    }

    pub(crate) fn exists(self) -> bool {
        unsafe { IsWindow(Some(HWND(self.hwnd as *mut _))).as_bool() }
    }
}

/// 印を付けた側が所有し、別名や接続が不要になったときに外す。
#[derive(Debug, PartialEq, Eq)]
pub struct WindowMarker(pub WindowInstance);

impl WindowMarker {
    pub fn attach(hwnd: HWND) -> Result<Self> {
        let cookie = (GUID::new()?.to_u128() as usize).max(1);
        unsafe {
            SetPropW(
                hwnd,
                w!("Waypoint.Instance"),
                Some(HANDLE(cookie as *mut _)),
            )?
        };
        Ok(Self(WindowInstance {
            hwnd: hwnd.0 as isize,
            cookie,
        }))
    }
}

impl Drop for WindowMarker {
    fn drop(&mut self) {
        if self.0.is_current() {
            unsafe {
                let _ = RemovePropW(HWND(self.0.hwnd as *mut _), w!("Waypoint.Instance"));
            }
        }
    }
}
