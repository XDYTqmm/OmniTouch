//! 文件作用：完成应用启动初始化、窗口类注册、主窗口创建和消息循环。

// 发布模式下隐藏控制台窗口，调试模式保留控制台便于排查问题。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod app_state;
pub mod config;

pub mod input;
pub mod core;
pub mod ui;

use crate::core::wndprocs::{child_wndproc, main_wndproc, overlay_wndproc};
use crate::ui::{apply_window_attributes, create_menu_buttons};
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::System::Com::*,
    Win32::System::LibraryLoader::*,
    Win32::UI::Controls::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::HiDpi::*,
    Win32::UI::Input::KeyboardAndMouse::*,
};

/// 初始化 Windows 运行环境并进入主消息循环。
fn main() -> Result<()> {
    unsafe {
        // 尽早启用每显示器 DPI 感知，避免后续窗口尺寸计算失真。
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // 初始化当前线程的 COM 环境，供窗口与系统组件使用。
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&icex);

        let instance = GetModuleHandleW(None)?;

        // 注册主窗口、子窗口和悬浮层三类窗口过程。
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("OmniTouch-MainWindow"),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_wndproc),
            hbrBackground: CreateSolidBrush(COLORREF(0xFFFFFF)),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let child_wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("OmniTouch-ChildWindow"),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(child_wndproc),
            // 使用系统窗口底色，方便原生控件接管背景绘制。
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut std::ffi::c_void),
            ..Default::default()
        };
        RegisterClassW(&child_wc);

        let overlay_wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("OmniTouch-OverlayWindow"),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wndproc),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        RegisterClassW(&overlay_wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let w = 420;
        let h = 420;
        let x = (screen_w - w) / 2;
        let y = (screen_h - h) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            w!("OmniTouch-MainWindow"),
            w!("OmniTouch - 全域触控"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x, y, w, h,
            None,
            None,
            HINSTANCE(instance.0),
            None,
        )?;

        apply_window_attributes(hwnd)?;

        create_menu_buttons(hwnd, instance)?;

        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);

        let _overlay_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("OmniTouch-OverlayWindow"),
            w!("OmniTouch Overlay"),
            WS_POPUP,
            0, 0, screen_width, screen_height,
            None,
            None,
            HINSTANCE(instance.0),
            None,
        )?;

        // 提前把焦点还给主窗口，避免悬浮层抢占输入。
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        Ok(())
    }
}
