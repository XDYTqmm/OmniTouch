//! 文件作用：封装 ViGEm 虚拟手柄初始化与状态同步。

use std::ffi::c_void;
use std::ptr;
use std::sync::Once;
use std::sync::atomic::{AtomicU8, AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONWARNING};

use crate::app_state::{VirtualButton, ButtonVariant};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct XUSB_REPORT {
    pub wButtons: u16,
    pub bLeftTrigger: u8,
    pub bRightTrigger: u8,
    pub sThumbLX: i16,
    pub sThumbLY: i16,
    pub sThumbRX: i16,
    pub sThumbRY: i16,
}

type PVIGEMCLIENT = *mut c_void;
type PVIGEMTARGET = *mut c_void;

const VIGEM_ERROR_NONE: u32 = 0x20000000;

extern "C" {
    fn vigem_alloc() -> PVIGEMCLIENT;
    fn vigem_free(client: PVIGEMCLIENT);
    fn vigem_connect(client: PVIGEMCLIENT) -> u32;
    fn vigem_disconnect(client: PVIGEMCLIENT);
    fn vigem_target_x360_alloc() -> PVIGEMTARGET;
    fn vigem_target_free(target: PVIGEMTARGET);
    fn vigem_target_add(client: PVIGEMCLIENT, target: PVIGEMTARGET) -> u32;
    fn vigem_target_remove(client: PVIGEMCLIENT, target: PVIGEMTARGET) -> u32;
    fn vigem_target_x360_update(client: PVIGEMCLIENT, target: PVIGEMTARGET, report: XUSB_REPORT) -> u32;
}

pub struct ViGEmClient {
    client: PVIGEMCLIENT,
    target: PVIGEMTARGET,
}

static mut GLOBAL: *mut ViGEmClient = ptr::null_mut();
static INIT: Once = Once::new();
static INIT_RESULT: AtomicU8 = AtomicU8::new(0);
static NOTIFIED: AtomicBool = AtomicBool::new(false);

impl ViGEmClient {
    /// 延迟初始化全局 ViGEm 客户端，并返回可复用实例。
    pub fn try_init() -> Option<&'static ViGEmClient> {
        unsafe {
            INIT.call_once(|| {
                let client = vigem_alloc();
                if client.is_null() {
                    INIT_RESULT.store(4, Ordering::Relaxed);
                    return;
                }
                let err = vigem_connect(client);
                if err != VIGEM_ERROR_NONE {
                    vigem_free(client);
                    INIT_RESULT.store(5, Ordering::Relaxed);
                    return;
                }
                let target = vigem_target_x360_alloc();
                if target.is_null() {
                    vigem_free(client);
                    INIT_RESULT.store(6, Ordering::Relaxed);
                    return;
                }
                let add_err = vigem_target_add(client, target);
                if add_err != VIGEM_ERROR_NONE {
                    vigem_target_free(target);
                    vigem_free(client);
                    INIT_RESULT.store(7, Ordering::Relaxed);
                    return;
                }

                let obj = Box::new(ViGEmClient { client, target });
                GLOBAL = Box::into_raw(obj);
                INIT_RESULT.store(1, Ordering::Relaxed);
            });

            if GLOBAL.is_null() { None } else { Some(&*GLOBAL) }
        }
    }
}

/// 返回虚拟手柄客户端是否已成功初始化。
pub fn vigem_available() -> bool {
    INIT_RESULT.load(Ordering::Relaxed) == 1
}

/// 在初始化失败时只弹出一次驱动告警。
fn notify_failure_once() {
    if NOTIFIED.swap(true, Ordering::Relaxed) { return; }
    let code = INIT_RESULT.load(Ordering::Relaxed);
    let msg = match code {
        4 => "ViGEm 客户端内存分配失败。",
        5 => "ViGEm 客户端连接失败 (可能是总线驱动未启动)。",
        6 => "ViGEm 手柄目标分配失败。",
        7 => "ViGEm 接入系统失败。",
        _ => "ViGEm 虚拟手柄初始化失败。",
    };
    let wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "驱动缺失\0".encode_utf16().collect();
    unsafe { MessageBoxW(None, PCWSTR(wide.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONWARNING); }
}

impl Drop for ViGEmClient {
    /// 释放手柄目标并断开 ViGEm 连接。
    fn drop(&mut self) {
        unsafe {
            let _ = vigem_target_remove(self.client, self.target);
            vigem_target_free(self.target);
            vigem_disconnect(self.client);
            vigem_free(self.client);
        }
    }
}

/// 主动建立虚拟手柄连接，并在失败时提示用户。
pub fn connect_gamepad() {
    let _ = ViGEmClient::try_init();
    if !vigem_available() {
        notify_failure_once();
    }
}

/// 将当前虚拟按键状态同步为 Xbox 手柄报告。
pub fn sync_gamepad(buttons: &[VirtualButton]) {
    if !vigem_available() { return; }
    let client = match ViGEmClient::try_init() {
        Some(c) => c,
        None => return,
    };

    let mut r = XUSB_REPORT::default();

    for btn in buttons {
        if btn.is_pressed {
            if btn.variant == ButtonVariant::Normal {
                let mask = match btn.key_code {
                    x if x == VK_GAMEPAD_A.0 => 0x1000u16,
                    x if x == VK_GAMEPAD_B.0 => 0x2000u16,
                    x if x == VK_GAMEPAD_X.0 => 0x4000u16,
                    x if x == VK_GAMEPAD_Y.0 => 0x8000u16,
                    x if x == VK_GAMEPAD_LEFT_SHOULDER.0 => 0x0100u16,
                    x if x == VK_GAMEPAD_RIGHT_SHOULDER.0 => 0x0200u16,
                    x if x == VK_GAMEPAD_DPAD_UP.0 => 0x0001u16,
                    x if x == VK_GAMEPAD_DPAD_DOWN.0 => 0x0002u16,
                    x if x == VK_GAMEPAD_DPAD_LEFT.0 => 0x0004u16,
                    x if x == VK_GAMEPAD_DPAD_RIGHT.0 => 0x0008u16,
                    x if x == VK_GAMEPAD_MENU.0 => 0x0010u16,
                    x if x == VK_GAMEPAD_VIEW.0 => 0x0020u16,
                    x if x == VK_GAMEPAD_LEFT_THUMBSTICK_BUTTON.0 => 0x0040u16,
                    x if x == VK_GAMEPAD_RIGHT_THUMBSTICK_BUTTON.0 => 0x0080u16,
                    _ => 0u16,
                };
                r.wButtons |= mask;
            } else if btn.variant == ButtonVariant::Trigger {
                let val = (btn.joystick_val.1 * 255.0).clamp(0.0, 255.0) as u8;
                if btn.label == "LT" { r.bLeftTrigger = val; }
                else if btn.label == "RT" { r.bRightTrigger = val; }
            } else if btn.variant == ButtonVariant::Joystick {
                let jx = (btn.joystick_val.0 * 32767.0).clamp(-32768.0, 32767.0) as i16;
                let jy = (-btn.joystick_val.1 * 32767.0).clamp(-32768.0, 32767.0) as i16;
                
                if btn.label == "L-Stick" {
                    r.sThumbLX = jx;
                    r.sThumbLY = jy;
                } else if btn.label == "R-Stick" {
                    r.sThumbRX = jx;
                    r.sThumbRY = jy;
                }
            }
        }
    }

    unsafe {
        let _ = vigem_target_x360_update(client.client, client.target, r);
    }
}
