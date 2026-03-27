//! 文件作用：集中处理主窗口、编辑窗口、侧边栏和悬浮层的 Windows 消息。

// 从配置模块导入必要的函数和全局状态
use crate::config::{ensure_configs_dir, get_config_directory, CONFIGS, CONFIG_SELECTED};

// 从 app_state 模块导入状态管理
use crate::app_state::{AppState, ProgramMode};

// 从 UI 模块导入主菜单按钮创建函数
use crate::ui::create_menu_buttons;

// 从窗口模块导入窗口创建和刷新函数
use crate::ui::panels::{
    create_config_window, refresh_config_list, start_rename, CONFIG_WINDOW,
    IDB_DELETE_CONFIG, IDB_LOAD_CONFIG, IDB_NEW_CONFIG, IDB_COPY_CONFIG, IDB_OPEN_FOLDER, IDB_RENAME_CONFIG,
    IDL_CONFIG_LIST, RENAME_EDIT_HWND, RENAME_INDEX, RENAME_ORIGINAL_NAME, RENAME_COMMITTING,
    is_valid_filename,
};

// 导入键盘和鼠标 API（用于虚拟键码）
use windows::Win32::UI::Input::KeyboardAndMouse::*;

// 导入 Windows Shell API（用于 SetWindowSubclass 和 RemoveWindowSubclass）
use windows::Win32::UI::Shell::*;

// 导入 Windows 消息和窗口管理 API
use windows::Win32::UI::WindowsAndMessaging::*;
// 导入 Windows UI Controls API（用于 WC_LISTBOXW、WC_BUTTONW 等）
use windows::Win32::UI::Controls::*;

const LVN_ITEMCHANGED: i32 = -101;
const NM_DBLCLK: i32 = -3;
const LVIS_SELECTED: u32 = 0x0002;
const LVIF_STATE: u32 = 0x00000008;

#[repr(C)]
struct NMLISTVIEW_LOCAL {
    hdr: NMHDR,
    iItem: i32,
    iSubItem: i32,
    uNewState: u32,
    uOldState: u32,
    uChanged: u32,
    ptAction: POINT,
    lParam: LPARAM,
}

use std::fs;
use std::cell::RefCell;
use std::sync::atomic::Ordering;

// 导入 Windows API 核心类型
use windows::core::*;

// 导入 Windows 基础类型（如 HWND、WPARAM、LRESULT 等）
use windows::Win32::Foundation::*;

// 导入 GDI 相关 API
use windows::Win32::Graphics::Gdi::*;

// 导入系统库加载器 API（用于获取模块句柄）
use windows::Win32::System::LibraryLoader::*;

/// 线程本地应用状态，供各窗口过程共享。
thread_local! { pub static APP_STATE: RefCell<AppState> = RefCell::new(AppState::new()); }

// 低级鼠标钩子用于拦截悬浮键盘区域内的物理点击。
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowsHookExW, CallNextHookEx, WH_MOUSE_LL, MSLLHOOKSTRUCT, HHOOK, HC_ACTION,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WindowFromPoint, GetWindowThreadProcessId, GetAncestor, GA_ROOT
};

thread_local! {
    static MOUSE_HOOK: RefCell<HHOOK> = RefCell::new(HHOOK::default());
}

const TOUCH_MOUSE_SIGNATURE: usize = 0xFF51_5700;
const TOUCH_MOUSE_SIGNATURE_MASK: usize = 0xFFFF_FF00;

fn is_synthetic_touch_mouse_message() -> bool {
    unsafe {
        let extra = GetMessageExtraInfo().0 as usize;
        (extra & TOUCH_MOUSE_SIGNATURE_MASK) == TOUCH_MOUSE_SIGNATURE
    }
}

fn is_injected_mouse_message() -> bool {
    unsafe { GetMessageExtraInfo().0 as usize == crate::input::handler::INJECTED_INPUT_SIGNATURE }
}

unsafe fn keep_window_topmost(hwnd: HWND) {
    if IsWindow(hwnd).as_bool() {
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

unsafe fn scroll_settings_surface(hwnd: HWND, delta_y: i32) -> bool {
    let h_settings = GetDlgItem(hwnd, IDC_SETTINGS_BACK).unwrap_or_default();
    let h_about = GetDlgItem(hwnd, IDC_ABOUT_BACK).unwrap_or_default();

    let is_settings = !h_settings.is_invalid() && IsWindowVisible(h_settings).as_bool();
    let is_about = !h_about.is_invalid() && IsWindowVisible(h_about).as_bool();
    if !is_settings && !is_about {
        return false;
    }

    let current_scroll = GetPropW(hwnd, w!("SettingsScrollY")).0 as i32;
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let window_h = rect.bottom - rect.top;
    let content_h = if is_about { 330 } else { 410 };
    let max_scroll = (content_h - window_h).max(0);
    let new_scroll = (current_scroll - delta_y).clamp(0, max_scroll);

    if new_scroll != current_scroll {
        let _ = SetPropW(hwnd, w!("SettingsScrollY"), HANDLE(new_scroll as isize as *mut _));
        SendMessageW(hwnd, WM_SIZE, WPARAM(0), LPARAM(0));
    }
    true
}

unsafe extern "system" fn settings_touch_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref_data: usize,
) -> LRESULT {
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERUPDATE: u32 = 0x0245;
    const DRAG_THRESHOLD: i32 = 8;

    match msg {
        WM_POINTERDOWN => {
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let _ = SetPropW(hwnd, w!("TouchPanStartY"), HANDLE(y as isize as *mut _));
            let _ = SetPropW(hwnd, w!("TouchPanLastY"), HANDLE(y as isize as *mut _));
            let _ = SetPropW(hwnd, w!("TouchPanDragging"), HANDLE(0 as _));
            let _ = SetCapture(hwnd);
        }
        WM_POINTERUPDATE => {
            if GetCapture() == hwnd {
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let start_y = GetPropW(hwnd, w!("TouchPanStartY")).0 as i32;
                let last_y = GetPropW(hwnd, w!("TouchPanLastY")).0 as i32;
                let mut dragging = GetPropW(hwnd, w!("TouchPanDragging")).0 as isize != 0;

                if !dragging && (y - start_y).abs() >= DRAG_THRESHOLD {
                    dragging = true;
                    let _ = SetPropW(hwnd, w!("TouchPanDragging"), HANDLE(1 as _));
                }

                if dragging {
                    let parent = GetParent(hwnd).unwrap_or_default();
                    if !parent.is_invalid() {
                        let _ = scroll_settings_surface(parent, y - last_y);
                    }
                    let _ = SetPropW(hwnd, w!("TouchPanLastY"), HANDLE(y as isize as *mut _));
                    return LRESULT(0);
                }
            }
        }
        WM_POINTERUP => {
            let dragging = GetPropW(hwnd, w!("TouchPanDragging")).0 as isize != 0;
            if GetCapture() == hwnd {
                let _ = ReleaseCapture();
            }
            let _ = RemovePropW(hwnd, w!("TouchPanStartY"));
            let _ = RemovePropW(hwnd, w!("TouchPanLastY"));
            let _ = RemovePropW(hwnd, w!("TouchPanDragging"));
            if dragging {
                return LRESULT(0);
            }
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(settings_touch_subclass_proc), 0x5354);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

unsafe fn enable_settings_touch_pan(hwnd: HWND) {
    let _ = SetWindowSubclass(hwnd, Some(settings_touch_subclass_proc), 0x5354, 0);
}

/// 确保当前线程已安装用于悬浮键盘的低级鼠标钩子。
fn ensure_mouse_hook() {
    MOUSE_HOOK.with(|h| {
        if h.borrow().is_invalid() {
            unsafe {
                let hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(osk_mouse_hook_proc),
                    GetModuleHandleW(None).unwrap_or_default(),
                    0,
                ).unwrap();
                *h.borrow_mut() = hook;
            }
        }
    });
}

/// 处理系统级鼠标钩子回调，拦截悬浮键盘区域内的点击。
unsafe extern "system" fn osk_mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let msg_id = wparam.0 as u32;
        if msg_id == WM_LBUTTONDOWN || msg_id == WM_LBUTTONUP {
            let hook_struct = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let mut eat = false;
            let mut target_vk = None;
            let is_down = msg_id == WM_LBUTTONDOWN;

            let hwnd_under_mouse = WindowFromPoint(hook_struct.pt);
            let target_hwnd = GetAncestor(hwnd_under_mouse, GA_ROOT);
            
            let mut process_id = 0u32;
            GetWindowThreadProcessId(target_hwnd, Some(&mut process_id));

            if process_id == std::process::id() {
                APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        if state.osk_visible {
                            let mut client_pt = hook_struct.pt;
                            ScreenToClient(target_hwnd, &mut client_pt);

                            let osk_rect = state.osk_rect;
                            let title_rect = RECT {
                                left: osk_rect.left,
                                top: osk_rect.top,
                                right: osk_rect.right,
                                bottom: osk_rect.top + 50,
                            };

                            if !PtInRect(&title_rect, client_pt).as_bool() 
                                && PtInRect(&osk_rect, client_pt).as_bool() {
                                eat = true;
                                for btn in &mut state.osk_buttons {
                                    let r: RECT = btn.rect.into();
                                    let abs_r = RECT {
                                        left: osk_rect.left + r.left,
                                        top: osk_rect.top + r.top,
                                        right: osk_rect.left + r.right,
                                        bottom: osk_rect.top + r.bottom,
                                    };
                                    if PtInRect(&abs_r, client_pt).as_bool() {
                                        target_vk = Some(btn.key_code);
                                        btn.is_pressed = is_down;
                                        break;
                                    }
                                }
                                InvalidateRect(target_hwnd, None, FALSE);
                            }
                        }
                    }
                });
            }

            if eat {
                if let Some(vk) = target_vk {
                    crate::input::handler::simulate_key(VIRTUAL_KEY(vk), is_down);
                }
                return LRESULT(1);
            }
        }
    }
    
    let hook = MOUSE_HOOK.with(|h| *h.borrow());
    CallNextHookEx(hook, code, wparam, lparam)
}
// ==========================================

/// 初始化编辑模式状态，并同步首帧编辑界面。
pub unsafe fn init_edit_state(edit_hwnd: HWND) {
    APP_STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.mode = ProgramMode::Editing;
            crate::ui::render::force_redraw(edit_hwnd, &mut s);
            sync_sidebar_list_from_edit(edit_hwnd, &mut s);
        }
    });
}

/// 从编辑窗口取出侧边栏句柄并刷新树状列表。
unsafe fn sync_sidebar_list_from_edit(edit_hwnd: HWND, state: &mut AppState) {
    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
    if !sidebar_handle.is_invalid() {
        let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
        if IsWindow(sidebar_hwnd).as_bool() {
            crate::ui::sync_sidebar_list(sidebar_hwnd, state);
        }
    }
}

// =============================================================================
// 子类化函数 - 用于处理编辑框的键盘和焦点消息
// =============================================================================

/// 处理重命名输入框的键盘和焦点消息。
pub unsafe extern "system" fn edit_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref_data: usize,
) -> LRESULT {
    println!("[DEBUG edit_subclass_proc] 收到消息: msg={}", msg);
    
    match msg {
        // 重命名框获得焦点时，同步唤起悬浮键盘。
        WM_SETFOCUS => {
            crate::core::wndprocs::APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    state.osk_visible = true;
                    if let Ok(overlay) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                        if IsWindow(overlay).as_bool() {
                            let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
                            let _ = SetWindowPos(overlay, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                            crate::ui::render::force_redraw(overlay, &mut state);
                        }
                    }
                }
            });
        }
        // 处理按下 Enter 键
        WM_KEYDOWN => {
            println!("[DEBUG edit_subclass_proc] WM_KEYDOWN: wparam={}", wparam.0);
            if wparam.0 == VK_RETURN.0 as usize {
                println!("[DEBUG edit_subclass_proc] 检测到 Enter 键");
                commit_rename(hwnd);
                return LRESULT(0);
            } else if wparam.0 == VK_ESCAPE.0 as usize {
                println!("[DEBUG edit_subclass_proc] 检测到 ESC 键");
                // ESC 直接放弃重命名并清理状态。
                let _ = DestroyWindow(hwnd);
                *RENAME_INDEX.lock().unwrap() = None;
                *RENAME_EDIT_HWND.lock().unwrap() = None;
                *RENAME_ORIGINAL_NAME.lock().unwrap() = None;
                *RENAME_COMMITTING.lock().unwrap() = false;
                return LRESULT(0);
            }
        }
        WM_KILLFOCUS => {
            println!("[DEBUG edit_subclass_proc] WM_KILLFOCUS");
            // 失去焦点时关闭悬浮键盘，并提交当前输入。
            crate::core::wndprocs::APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    state.osk_visible = false;
                    if let Ok(overlay) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                        if IsWindow(overlay).as_bool() {
                            crate::ui::render::force_redraw(overlay, &mut state);
                            let mode = crate::ui::CURRENT_MODE.load(std::sync::atomic::Ordering::SeqCst);
                            if mode == 0 {
                                let _ = ShowWindow(overlay, SW_HIDE);
                            }
                        }
                    }
                }
            });
            commit_rename(hwnd);
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            println!("[DEBUG edit_subclass_proc] WM_NCDESTROY");
            let _ = RemoveWindowSubclass(hwnd, Some(edit_subclass_proc), 100);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// 提交配置重命名，并同步刷新配置列表。
unsafe fn commit_rename(edit_hwnd: HWND) {
    println!("[DEBUG commit_rename] 开始执行");
    
    // 防止失焦和回车同时触发重复提交。
    let mut committing = RENAME_COMMITTING.lock().unwrap();
    if *committing {
        println!("[DEBUG commit_rename] 已在提交中，跳过");
        return;
    }
    *committing = true;
    drop(committing);
    
    let mut text_buf = [0u16; 260];
    let len = GetWindowTextW(edit_hwnd, &mut text_buf);
    let new_name = String::from_utf16_lossy(&text_buf[..len as usize]).trim().to_string();
    println!("[DEBUG commit_rename] 新名称: {}", new_name);

    if !new_name.is_empty() && is_valid_filename(&new_name) {
        let mut idx_guard = RENAME_INDEX.lock().unwrap();
        if let Some(idx) = *idx_guard {
            // 获取旧名称进行对比
            let should_rename = {
                let configs = CONFIGS.lock().unwrap();
                if idx < configs.len() && configs[idx].0 != new_name {
                    Some(configs[idx].1.clone())
                } else {
                    None
                }
            };

            if let Some(old_path) = should_rename {
                let configs_dir = get_config_directory();
                let new_path = configs_dir.join(format!("{}.json", new_name));

                let exists = {
                    let configs = CONFIGS.lock().unwrap();
                    configs.iter().any(|(n, p)| n == &new_name && p != &old_path)
                };

                if !exists && new_path != old_path {
                    println!("[DEBUG commit_rename] 执行重命名: {} -> {}", old_path.display(), new_path.display());
                    let _ = fs::rename(&old_path, &new_path);
                }
            }
        }
        *idx_guard = None;
    } else {
        println!("[DEBUG commit_rename] 名称无效或为空");
    }
    
    println!("[DEBUG commit_rename] 销毁编辑框");
    let _ = DestroyWindow(edit_hwnd);
    *RENAME_EDIT_HWND.lock().unwrap() = None;
    *RENAME_ORIGINAL_NAME.lock().unwrap() = None;
    *RENAME_COMMITTING.lock().unwrap() = false;
    
    // 提交后刷新列表，恢复界面状态。
    let config_window = {
        let cw = CONFIG_WINDOW.lock().unwrap();
        *cw
    };
    
    if let Some(hwnd_val) = config_window {
        let config_hwnd = HWND(hwnd_val as *mut std::ffi::c_void);
        if let Ok(list_hwnd) = GetDlgItem(config_hwnd, IDL_CONFIG_LIST) {
            println!("[DEBUG commit_rename] 刷新列表");
            refresh_config_list(list_hwnd, None);
        }
    }
}

// =============================================================================
// 主菜单按钮 ID 常量
// =============================================================================
// 这些 ID 用于标识主窗口中的按钮

/// 主菜单中的“开始映射”按钮 ID。
const IDM_START: i32 = 1001;

/// 主菜单中的“编辑按键”按钮 ID。
const IDM_EDIT: i32 = 1002;

/// 主菜单中的“选择配置”按钮 ID。
const IDM_CONFIG: i32 = 1003;

/// 主菜单中的“退出”按钮 ID。
const IDM_EXIT: i32 = 1004;
/// 主菜单中的“设置”按钮 ID。
const IDM_SETTINGS: i32 = 1005;

// 设置页与关于页使用的控件 ID。
const IDC_SETTINGS_BACK: i32 = 2001;
const IDC_SETTINGS_TITLE: i32 = 2002;
const IDC_SETTINGS_AUTOSTART: i32 = 2003;
const IDC_SETTINGS_OSK: i32 = 2004;
const IDC_SETTINGS_GAMEPAD: i32 = 2005;
const IDC_SETTINGS_TRAY: i32 = 2006;
const IDC_SETTINGS_ABOUT: i32 = 2007;

const IDC_ABOUT_BACK: i32 = 2008;
const IDC_ABOUT_TITLE: i32 = 2009;
const IDC_ABOUT_APPNAME: i32 = 2012;
const IDC_ABOUT_VERSION: i32 = 2013;
const IDC_ABOUT_AUTHOR: i32 = 2014;
const IDC_ABOUT_REPO: i32 = 2015;

pub const WM_TRAYICON: u32 = WM_USER + 1;

// =============================================================================
// 窗口过程函数
// =============================================================================

// =============================================================================
// 主窗口过程
// =============================================================================
// main_wndproc 处理主窗口收到的所有消息
// 
// Windows 应用程序的工作原理：
// 1. 程序注册一个窗口类（包含窗口过程函数指针）
// 2. 创建窗口时，系统会调用指定的窗口过程函数
// 3. 所有与该窗口相关的事件（点击、大小变化、关闭等）都会发送到窗口过程
// 4. 窗口过程根据消息类型执行相应的操作
// 
// # 参数说明
// - hwnd: 窗口句柄，标识发送消息的窗口
// - message: 消息类型（WM_xxx 常量）
// - wparam: 附加信息（含义取决于消息类型）
// - lparam: 附加信息（含义取决于消息类型）
// 
// # 返回值
// 返回 LRESULT 类型，表示消息处理的结果
// 大多数消息返回 0 表示处理成功
/// 主窗口过程，负责菜单交互、托盘逻辑和主流程切换。
pub unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // 使用 match 表达式处理不同的消息类型
    match message {
        // =====================================================================
        // WM_ERASEBKGND - 接管主窗口背景擦除，动态适配深色模式
        // =====================================================================
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut _);
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);

            let bg_color = if is_dark_mode() { 0x00202020 } else { 0x00F3F3F3 };
            let brush = CreateSolidBrush(COLORREF(bg_color));

            FillRect(hdc, &rect, brush);
            DeleteObject(HGDIOBJ(brush.0 as _));

            return LRESULT(1);
        }

        // =====================================================================
        // WM_CTLCOLORBTN / WM_CTLCOLORSTATIC - 动态改变控件的文字与底色
        // =====================================================================
        WM_CTLCOLORBTN | WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut _);
            let _ = SetBkMode(hdc, TRANSPARENT);

            if is_dark_mode() {
                let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
                let _ = SetDCBrushColor(hdc, COLORREF(0x00202020));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            } else {
                let _ = SetTextColor(hdc, COLORREF(0x00000000));
                let _ = SetDCBrushColor(hdc, COLORREF(0x00F3F3F3));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
        }

        // =====================================================================
        // WM_GETMINMAXINFO - 窗口大小改变前发送
        // =====================================================================
        // 这个消息在窗口大小改变之前发送，允许程序设置窗口的最大和最小尺寸限制
        // 
        // lparam 指向 MINMAXINFO 结构，包含：
        // - ptReserved: 保留
        // - ptMaxSize: 窗口的最大尺寸（如果设置了 WS_MAXIMIZE 样式）
        // - ptMaxPosition: 窗口最大化时的位置
        // - ptMinTrackSize: 窗口拖动时的最小尺寸
        // - ptMaxTrackSize: 窗口拖动时的最大尺寸
        WM_GETMINMAXINFO => {
            // 获取 MINMAXINFO 结构的指针
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            
            // 设置最小跟踪尺寸
            // 这限制了用户拖动窗口时能缩到的最小大小
            mmi.ptMinTrackSize.x = 420;  // 最小宽度 420 像素
            mmi.ptMinTrackSize.y = 420;  // 最小高度 420 像素
            
            LRESULT(0)  // 返回 0 表示消息已处理
        }

        // =====================================================================
        // WM_COMMAND - 用户操作控件时发送
        // =====================================================================
        // 当用户点击按钮、选择菜单项等操作时，Windows 会向父窗口发送此消息
        // 
        // wparam 参数包含：
        // - 低 16 位 (wparam.0 & 0xFFFF): 控件 ID 或菜单 ID
        // - 高 16 位 (wparam.0 >> 16): 通知码
        // 
        // 对于按钮，通知码 BN_CLICKED (0) 表示按钮被点击
        WM_COMMAND => {
            // 从 wparam 中提取控件 ID（低 16 位）
            let control_id = (wparam.0 & 0xFFFF) as i32;
            // 从 wparam 中提取通知码（高 16 位）
            let notification_code = (wparam.0 >> 16) as i32;
            
            // 只处理按钮点击事件（BN_CLICKED）
            if notification_code == BN_CLICKED as i32 {
                // 获取模块实例句柄
                if let Ok(instance) = GetModuleHandleW(None) {
                    // 根据按钮 ID 执行相应的操作
                    match control_id {
                        // "开始/暂停 映射" 按钮
                        IDM_START => {
                            let is_active = crate::ui::MAPPING_ACTIVE.load(Ordering::SeqCst);
                            
                            if is_active {
                                // 映射中点击按钮时，先暂停映射并隐藏悬浮层。
                                crate::ui::MAPPING_ACTIVE.store(false, Ordering::SeqCst);
                                crate::ui::CURRENT_MODE.store(0, Ordering::SeqCst);
                                
                                let _ = create_menu_buttons(hwnd, instance);
                                let _ = InvalidateRect(hwnd, None, TRUE);

                                if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                    if IsWindow(overlay_hwnd).as_bool() {
                                        let _ = ShowWindow(overlay_hwnd, SW_HIDE);
                                        KillTimer(overlay_hwnd, 100);
                                        
                                        APP_STATE.with(|s| {
                                            if let Ok(mut state) = s.try_borrow_mut() {
                                                // 暂停时立即释放所有按键，避免残留按下状态。
                                                state.emergency_release_all_keys();
                                                state.mode = ProgramMode::Menu;
                                            }
                                        });
                                    }
                                }
                            } else {
                                // 未映射时点击按钮，开始运行映射流程。
                                
                                // 未选择配置时不允许直接进入运行态。
                                let has_config = APP_STATE.with(|s| {
                                    if let Ok(state) = s.try_borrow() {
                                        state.config_selected.is_some()
                                    } else { false }
                                });

                                if !has_config {
                                    let msg = "还没有选中任何按键方案！\n\n请先点击「编辑按键」创建一个新方案，或在「选择配置」中选择一个。";
                                    let msg_wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
                                    MessageBoxW(hwnd, PCWSTR(msg_wide.as_ptr()), w!("提示"), MB_OK | MB_ICONWARNING);
                                    return LRESULT(0);
                                }
                                crate::ui::MAPPING_ACTIVE.store(true, Ordering::SeqCst);
                                crate::ui::CURRENT_MODE.store(2, Ordering::SeqCst);
                                
                                // 仅在启用虚拟手柄时初始化驱动。
                                let use_gamepad = APP_STATE.with(|s| { s.try_borrow().map(|s| s.use_virtual_gamepad).unwrap_or(false) });
                                if use_gamepad {
                                    crate::input::vigem_wrapper::connect_gamepad();
                                }
                                
                                let _ = create_menu_buttons(hwnd, instance);
                                let _ = InvalidateRect(hwnd, None, TRUE);

                                if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                    if IsWindow(overlay_hwnd).as_bool() {
                                        let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
                                        keep_window_topmost(overlay_hwnd);
                                        SetTimer(overlay_hwnd, 100, 16, None);
                                        
                                        APP_STATE.with(|s| {
                                            if let Ok(mut state) = s.try_borrow_mut() {
                                                state.mode = ProgramMode::Running;
                                                unsafe {
                                                    crate::ui::render::force_redraw(overlay_hwnd, &mut state);
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                        // "编辑按键" 按钮
                        IDM_EDIT => {
                            eprintln!("[DEBUG] 点击了编辑按键按钮");
                            
                            if crate::ui::MAPPING_ACTIVE.load(Ordering::SeqCst) {
                                crate::ui::MAPPING_ACTIVE.store(false, Ordering::SeqCst);
                                crate::ui::CURRENT_MODE.store(0, Ordering::SeqCst);
                                let _ = create_menu_buttons(hwnd, instance);
                                let _ = InvalidateRect(hwnd, None, TRUE);
                                if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                    if IsWindow(overlay_hwnd).as_bool() {
                                        let _ = ShowWindow(overlay_hwnd, SW_HIDE);
                                        KillTimer(overlay_hwnd, 100);
                                    }
                                }
                                APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.emergency_release_all_keys();
                                        state.mode = ProgramMode::Menu;
                                    }
                                });
                            }
                            
                            APP_STATE.with(|s| {
                                if let Ok(mut state) = s.try_borrow_mut() {
                                    if state.config_selected.is_none() {
                                        if let Some(new_name) = state.new_config() {
                                            if let Some(idx) = state.configs.iter().position(|c| c.0 == new_name) {
                                                state.config_selected = Some(idx);
                                            }
                                        }
                                    }
                                }
                            });
                            crate::ui::panels::enter_edit_mode(hwnd, HINSTANCE(instance.0), Some(edit_wndproc), Some(sidebar_wndproc));
                        }
                        // "选择配置" 按钮
                        IDM_CONFIG => {
                            if let Ok(instance) = GetModuleHandleW(None) {
                                if crate::ui::MAPPING_ACTIVE.load(Ordering::SeqCst) {
                                    crate::ui::MAPPING_ACTIVE.store(false, Ordering::SeqCst);
                                    crate::ui::CURRENT_MODE.store(0, Ordering::SeqCst);
                                    let _ = create_menu_buttons(hwnd, instance);
                                    let _ = InvalidateRect(hwnd, None, TRUE);
                                    if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                        if IsWindow(overlay_hwnd).as_bool() {
                                            let _ = ShowWindow(overlay_hwnd, SW_HIDE);
                                            KillTimer(overlay_hwnd, 100);
                                        }
                                    }
                                    APP_STATE.with(|s| {
                                        if let Ok(mut state) = s.try_borrow_mut() {
                                            state.emergency_release_all_keys();
                                            state.mode = ProgramMode::Menu;
                                        }
                                    });
                                }

                            APP_STATE.with(|s| {
                                if let Ok(state) = s.try_borrow() {
                                        if let Some(state_idx) = state.config_selected {
                                            if state_idx < state.configs.len() {
                                                let name = state.configs[state_idx].0.trim_end_matches(".json");
                                                let configs = CONFIGS.lock().unwrap();
                                                if let Some(ui_idx) = configs.iter().position(|(n, _)| n == name) {
                                                    *CONFIG_SELECTED.lock().unwrap() = Some(ui_idx);
                                                }
                                            }
                                        }
                                    }
                                });
                                let _ = create_config_window(instance);
                            }
                        }
                        // "退出" 按钮
                        IDM_EXIT => {
                            // 销毁主窗口，这将触发 WM_DESTROY 消息
                            let _ = DestroyWindow(hwnd);
                        }
                        
                        IDM_SETTINGS => {
                            let _ = SetPropW(hwnd, w!("SettingsScrollY"), HANDLE(0 as _));
                            
                            let buttons = crate::ui::MENU_BUTTONS.lock().unwrap();
                            for &btn in buttons.iter() {
                                let _ = ShowWindow(HWND(btn as *mut std::ffi::c_void), SW_HIDE);
                            }
                            drop(buttons);

                            let mut rect = RECT::default();
                            let _ = GetClientRect(hwnd, &mut rect);
                            let center_x = (rect.right - rect.left) / 2;

                            let (auto, osk, pad, tray) = APP_STATE.with(|s| {
                                let st = s.borrow();
                                (st.auto_start, st.use_system_osk, st.use_virtual_gamepad, st.minimize_to_tray)
                            });

                            let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32 | 0x0100 | 0x0C00);
                            let title_style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32 | 0x0C00);

                            let h_back = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("  < 返回"),
                                style, 20, 20, 80, 35, hwnd, HMENU(IDC_SETTINGS_BACK as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_back);
                            enable_settings_touch_pan(h_back);

                            let h_title = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("软件设置"),
                                title_style, center_x - 60, 25, 120, 35, hwnd, HMENU(IDC_SETTINGS_TITLE as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_title);
                            enable_settings_touch_pan(h_title);

                            let auto_str = if auto { w!("   ✅    开机自启") } else { w!("   ❌    开机自启") };
                            let h_auto = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, auto_str,
                                style, center_x - 160, 90, 320, 45, hwnd, HMENU(IDC_SETTINGS_AUTOSTART as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_auto);
                            enable_settings_touch_pan(h_auto);

                            let osk_str = if osk { w!("   ✅    系统屏幕键盘") } else { w!("   ❌    系统屏幕键盘") };
                            let h_osk = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, osk_str,
                                style, center_x - 160, 150, 320, 45, hwnd, HMENU(IDC_SETTINGS_OSK as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_osk);
                            enable_settings_touch_pan(h_osk);

                            let pad_str = if pad { w!("   ✅    虚拟 Xbox 手柄") } else { w!("   ❌    虚拟 Xbox 手柄") };
                            let h_gamepad = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, pad_str,
                                style, center_x - 160, 210, 320, 45, hwnd, HMENU(IDC_SETTINGS_GAMEPAD as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_gamepad);
                            enable_settings_touch_pan(h_gamepad);

                            let tray_str = if tray { w!("   ✅    关闭最小化到托盘") } else { w!("   ❌    关闭最小化到托盘") };
                            let h_tray = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, tray_str,
                                style, center_x - 160, 270, 320, 45, hwnd, HMENU(IDC_SETTINGS_TRAY as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_tray);
                            enable_settings_touch_pan(h_tray);

                            let h_about = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("    关于 OmniTouch                                                            >"),
                                style, center_x - 160, 330, 320, 45, hwnd, HMENU(IDC_SETTINGS_ABOUT as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_about);
                            enable_settings_touch_pan(h_about);
                        }

                        // 处理设置面板内按钮和开关的点击交互。
                        IDC_SETTINGS_BACK => {
                            for id in [IDC_SETTINGS_BACK, IDC_SETTINGS_TITLE, IDC_SETTINGS_AUTOSTART, IDC_SETTINGS_OSK, IDC_SETTINGS_GAMEPAD, IDC_SETTINGS_TRAY, IDC_SETTINGS_ABOUT] {
                                if let Ok(h) = GetDlgItem(hwnd, id) {
                                    let _ = DestroyWindow(h);
                                }
                            }
                            let buttons = crate::ui::MENU_BUTTONS.lock().unwrap();
                            for &btn in buttons.iter() {
                                let _ = ShowWindow(HWND(btn as *mut std::ffi::c_void), SW_SHOW);
                            }
                        }
                        IDC_SETTINGS_AUTOSTART => {
                            if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_AUTOSTART) {
                                let is_on = APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.auto_start = !state.auto_start;
                                        crate::app_state::set_auto_start(state.auto_start);
                                        state.auto_start
                                    } else { false }
                                });
                                let text = if is_on { w!("   ✅    开机自启") } else { w!("   ❌    开机自启") };
                                let _ = SetWindowTextW(h, text);
                            }
                        }
                        IDC_SETTINGS_OSK => {
                            if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_OSK) {
                                let is_on = APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.use_system_osk = !state.use_system_osk;
                                        state.save_global_settings();
                                        state.use_system_osk
                                    } else { false }
                                });
                                let text = if is_on { w!("   ✅    系统屏幕键盘") } else { w!("   ❌    系统屏幕键盘") };
                                let _ = SetWindowTextW(h, text);
                            }
                        }
                        IDC_SETTINGS_GAMEPAD => {
                            if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_GAMEPAD) {
                                let is_on = APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.use_virtual_gamepad = !state.use_virtual_gamepad;
                                        state.save_global_settings();
                                        state.use_virtual_gamepad
                                    } else { false }
                                });
                                let text = if is_on { w!("   ✅    虚拟 Xbox 手柄") } else { w!("   ❌    虚拟 Xbox 手柄") };
                                let _ = SetWindowTextW(h, text);
                            }
                        }
                        IDC_SETTINGS_TRAY => {
                            if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_TRAY) {
                                let is_on = APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.minimize_to_tray = !state.minimize_to_tray;
                                        state.save_global_settings();
                                        state.minimize_to_tray
                                    } else { false }
                                });
                                let text = if is_on { w!("   ✅    关闭最小化到托盘") } else { w!("   ❌    关闭最小化到托盘") };
                                let _ = SetWindowTextW(h, text);
                            }
                        }

                        IDC_SETTINGS_ABOUT => {
                            let _ = SetPropW(hwnd, w!("SettingsScrollY"), HANDLE(0 as _));
                            
                            for id in [IDC_SETTINGS_BACK, IDC_SETTINGS_TITLE, IDC_SETTINGS_AUTOSTART, IDC_SETTINGS_OSK, IDC_SETTINGS_GAMEPAD, IDC_SETTINGS_TRAY, IDC_SETTINGS_ABOUT] {
                                if let Ok(h) = GetDlgItem(hwnd, id) { let _ = ShowWindow(h, SW_HIDE); }
                            }

                            let mut rect = RECT::default();
                            let _ = GetClientRect(hwnd, &mut rect);
                            let center_x = (rect.right - rect.left) / 2;

                            let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32 | 0x0100 | 0x0C00);
                            let title_style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32 | 0x0C00);

                            let h_back = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("  < 返回"),
                                style, 20, 20, 80, 35, hwnd, HMENU(IDC_ABOUT_BACK as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_back);
                            enable_settings_touch_pan(h_back);

                            let h_title = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("关于"),
                                title_style, center_x - 60, 25, 120, 35, hwnd, HMENU(IDC_ABOUT_TITLE as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_title);
                            enable_settings_touch_pan(h_title);

                            let h_appname = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("   软件名称            OmniTouch 全能触控"),
                                style, center_x - 160, 90, 320, 45, hwnd, HMENU(IDC_ABOUT_APPNAME as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_appname);
                            enable_settings_touch_pan(h_appname);

                            let h_version = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("   当前版本                                   v1.0.0"),
                                style, center_x - 160, 150, 320, 45, hwnd, HMENU(IDC_ABOUT_VERSION as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_version);
                            enable_settings_touch_pan(h_version);

                            let h_author = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("   核心作者                               青冥日月"),
                                style, center_x - 160, 210, 320, 45, hwnd, HMENU(IDC_ABOUT_AUTHOR as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_author);
                            enable_settings_touch_pan(h_author);

                            let h_repo = CreateWindowExW(
                                WINDOW_EX_STYLE::default(), WC_BUTTONW, w!("   开源代码                      前往 GitHub >"),
                                style, center_x - 160, 270, 320, 45, hwnd, HMENU(IDC_ABOUT_REPO as isize as *mut std::ffi::c_void), instance, None,
                            ).unwrap();
                            crate::ui::apply_modern_font(h_repo);
                            enable_settings_touch_pan(h_repo);
                        }

                        IDC_ABOUT_BACK => {
                            let _ = SetPropW(hwnd, w!("SettingsScrollY"), HANDLE(0 as _));
                            
                            for id in [IDC_ABOUT_BACK, IDC_ABOUT_TITLE, IDC_ABOUT_APPNAME, IDC_ABOUT_VERSION, IDC_ABOUT_AUTHOR, IDC_ABOUT_REPO] {
                                if let Ok(h) = GetDlgItem(hwnd, id) { let _ = DestroyWindow(h); }
                            }
                            for id in [IDC_SETTINGS_BACK, IDC_SETTINGS_TITLE, IDC_SETTINGS_AUTOSTART, IDC_SETTINGS_OSK, IDC_SETTINGS_GAMEPAD, IDC_SETTINGS_TRAY, IDC_SETTINGS_ABOUT] {
                                if let Ok(h) = GetDlgItem(hwnd, id) { let _ = ShowWindow(h, SW_SHOW); }
                            }
                            SendMessageW(hwnd, WM_SIZE, WPARAM(0), LPARAM(0));
                        }

                        IDC_ABOUT_REPO => {
                            let url: Vec<u16> = "https://github.com/XDYTqmm/OmniTouch\0".encode_utf16().collect();
                            let _ = ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOW);
                        }
                        // 未处理的按钮 ID
                        _ => {}
                    }
                }
            }
            LRESULT(0)
        }

        // =====================================================================
        // WM_CREATE - 窗口创建时，默默在右下角注册一个托盘图标
        // =====================================================================
        WM_CREATE => {
            let mut nid = windows::Win32::UI::Shell::NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<windows::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = windows::Win32::UI::Shell::NIF_ICON | windows::Win32::UI::Shell::NIF_MESSAGE | windows::Win32::UI::Shell::NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
            
            let tip = "OmniTouch 全域触控\0".encode_utf16().collect::<Vec<u16>>();
            for i in 0..tip.len().min(127) { nid.szTip[i] = tip[i]; }
            
            let _ = windows::Win32::UI::Shell::Shell_NotifyIconW(windows::Win32::UI::Shell::NIM_ADD, &nid);
            return LRESULT(0);
        }

        // =====================================================================
        // WM_CLOSE - 拦截点击右上角"X"的必杀技
        // =====================================================================
        WM_CLOSE => {
            let minimize = APP_STATE.with(|s| {
                if let Ok(state) = s.try_borrow() { state.minimize_to_tray } else { false }
            });

            if minimize {
                let _ = ShowWindow(hwnd, SW_HIDE);
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }

        // =====================================================================
        // WM_SETTINGCHANGE - 监听系统主题/深浅模式切换
        // =====================================================================
        WM_SETTINGCHANGE => {
            let _ = crate::ui::base::apply_window_attributes(hwnd);
            let _ = InvalidateRect(hwnd, None, TRUE);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }

        // =====================================================================
        // WM_TRAYICON - 处理用户在右下角托盘的点击行为
        // =====================================================================
        WM_TRAYICON => {
            let event = lparam.0 as u32 & 0xFFFF;
            if event == WM_LBUTTONUP {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            } else if event == WM_RBUTTONUP {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = SetForegroundWindow(hwnd);
                
                let hmenu = CreatePopupMenu().unwrap_or_default();
                let _ = AppendMenuW(hmenu, MF_STRING, 10001, w!("显示主界面"));
                let _ = AppendMenuW(hmenu, MF_STRING, 10002, w!("退出 OmniTouch"));
                
                let cmd = TrackPopupMenu(hmenu, TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, None).0 as usize;
                let _ = DestroyMenu(hmenu);
                
                if cmd == 10001 {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                } else if cmd == 10002 {
                    let _ = DestroyWindow(hwnd);
                }
            }
            return LRESULT(0);
        }

        // =====================================================================
        // WM_MOUSEWHEEL - 实现设置界面与关于界面的平滑滚动
        // =====================================================================
        WM_MOUSEWHEEL => {
            let h_settings = GetDlgItem(hwnd, IDC_SETTINGS_BACK).unwrap_or_default();
            let h_about = GetDlgItem(hwnd, IDC_ABOUT_BACK).unwrap_or_default();
            
            let is_settings = !h_settings.is_invalid() && IsWindowVisible(h_settings).as_bool();
            let is_about = !h_about.is_invalid() && IsWindowVisible(h_about).as_bool();
            
            if is_settings || is_about {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let current_scroll = GetPropW(hwnd, w!("SettingsScrollY")).0 as i32;
                let mut new_scroll = current_scroll - (delta / 120) * 40;
                
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let window_h = rect.bottom - rect.top;
                
                let content_h = if is_about { 330 } else { 410 }; 
                let max_scroll = (content_h - window_h).max(0);
                
                new_scroll = new_scroll.clamp(0, max_scroll);
                
                if new_scroll != current_scroll {
                    let _ = SetPropW(hwnd, w!("SettingsScrollY"), HANDLE(new_scroll as isize as *mut _));
                    SendMessageW(hwnd, WM_SIZE, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }

        // =====================================================================
        // WM_DESTROY - 真正的死亡降临
        // =====================================================================
        // 当窗口被销毁时发送此消息
        // 对于主窗口，这通常是用户点击关闭按钮或调用 DestroyWindow 时
        WM_DESTROY => {
            // 死前把右下角的图标拔掉，防止变成"幽灵图标"
            let mut nid = windows::Win32::UI::Shell::NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<windows::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            let _ = windows::Win32::UI::Shell::Shell_NotifyIconW(windows::Win32::UI::Shell::NIM_DELETE, &nid);

            // PostQuitMessage 会向消息队列发送 WM_QUIT 消息
            // 这会导致 GetMessageW 返回 false，从而退出消息循环
            // 参数 0 表示退出代码（通常 0 表示正常退出）
            PostQuitMessage(0);
            LRESULT(0)
        }

        // =====================================================================
        // WM_SIZE - 窗口大小改变后发送
        // =====================================================================
        // 当窗口大小改变后发送此消息
        // wparam 包含调整类型的标志：
        // - SIZE_RESTORED: 窗口从最大化/最小化恢复到正常大小
        // - SIZE_MINIMIZED: 窗口最小化
        // - SIZE_MAXIMIZED: 窗口最大化
        // - SIZE_MAXHIDE: 另一个窗口最大化（显示此窗口）
        // - SIZE_MAXSHOW: 另一个窗口从最大化恢复（显示此窗口）
        WM_SIZE => {
            if let Ok(instance) = GetModuleHandleW(None) {
                let _ = create_menu_buttons(hwnd, instance);

                let scroll_y = GetPropW(hwnd, w!("SettingsScrollY")).0 as i32;
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let center_x = (rect.right - rect.left) / 2;

                let h_settings_back = GetDlgItem(hwnd, IDC_SETTINGS_BACK).unwrap_or_default();
                if !h_settings_back.is_invalid() && IsWindowVisible(h_settings_back).as_bool() {
                    let buttons = crate::ui::MENU_BUTTONS.lock().unwrap();
                    for &btn in buttons.iter() { let _ = ShowWindow(HWND(btn as *mut std::ffi::c_void), SW_HIDE); }
                    drop(buttons);

                    // ！！返回按钮固定在左上角，并且通过 HWND_TOP 赋予其绝对置顶权，防止被滚动上来的选项遮盖！！
                    let _ = SetWindowPos(h_settings_back, HWND_TOP, 20, 20, 80, 35, SWP_SHOWWINDOW);
                    
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_TITLE) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 60, 25 - scroll_y, 120, 35, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_AUTOSTART) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 90 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_OSK) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 150 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_GAMEPAD) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 210 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_TRAY) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 270 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_SETTINGS_ABOUT) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 330 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                }

                let h_about_back = GetDlgItem(hwnd, IDC_ABOUT_BACK).unwrap_or_default();
                if !h_about_back.is_invalid() && IsWindowVisible(h_about_back).as_bool() {
                    let buttons = crate::ui::MENU_BUTTONS.lock().unwrap();
                    for &btn in buttons.iter() { let _ = ShowWindow(HWND(btn as *mut std::ffi::c_void), SW_HIDE); }
                    drop(buttons);

                    // 关于界面的返回按钮同样置顶
                    let _ = SetWindowPos(h_about_back, HWND_TOP, 20, 20, 80, 35, SWP_SHOWWINDOW);

                    if let Ok(h) = GetDlgItem(hwnd, IDC_ABOUT_TITLE) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 60, 25 - scroll_y, 120, 35, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_ABOUT_APPNAME) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 90 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_ABOUT_VERSION) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 150 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_ABOUT_AUTHOR) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 210 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                    if let Ok(h) = GetDlgItem(hwnd, IDC_ABOUT_REPO) {
                        let _ = SetWindowPos(h, HWND::default(), center_x - 160, 270 - scroll_y, 320, 45, SWP_NOZORDER);
                    }
                }
            }
            
            let mut tme = windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT::default();
            tme.cbSize = std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32;
            tme.dwFlags = windows::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE;
            tme.hwndTrack = hwnd;
            unsafe { windows::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut tme) };

            LRESULT(0)
        }

        // =====================================================================
        // 默认消息处理
        // =====================================================================
        // 对于未处理的消息，调用 DefWindowProcW
        // 这是 Windows 提供的默认窗口过程，负责处理系统标准行为
        // 例如：窗口移动、最大化、最小化、绘制等
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// =============================================================================
// 子窗口过程
// =============================================================================
// child_wndproc 处理所有子窗口的消息
// 包括：配置选择窗口、功能子窗口
// 
// 由于多个子窗口使用同一个窗口类，它们共享这个窗口过程
// 我们通过检查窗口句柄来区分不同的窗口
/// 子窗口过程，负责配置窗口与普通子窗口的消息分发。
pub unsafe extern "system" fn child_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let hdc = HDC(wparam.0 as *mut _);
            let _ = SetBkMode(hdc, TRANSPARENT);

            if crate::core::wndprocs::is_dark_mode() {
                let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
                let _ = SetDCBrushColor(hdc, COLORREF(0x00202020));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            } else {
                let _ = SetTextColor(hdc, COLORREF(0x00000000));
                let _ = SetDCBrushColor(hdc, COLORREF(0x00F3F3F3));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
        }

        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut _);
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);

            let bg_color = if crate::core::wndprocs::is_dark_mode() { 0x00202020 } else { 0x00F3F3F3 };
            let brush = CreateSolidBrush(COLORREF(bg_color));
            FillRect(hdc, &rect, brush);
            DeleteObject(HGDIOBJ(brush.0 as _));
            return LRESULT(1);
        }

        WM_NOTIFY => {
            let nmhdr = &*(lparam.0 as *const NMHDR);
            let is_config_window = { CONFIG_WINDOW.lock().unwrap().map_or(false, |h| h == hwnd.0 as isize) };
            
            if is_config_window && nmhdr.idFrom == IDL_CONFIG_LIST as usize {
                if nmhdr.code == LVN_ITEMCHANGED as u32 {
                    let nmlv = &*(lparam.0 as *const NMLISTVIEW_LOCAL);
                    
                    if (nmlv.uChanged & LVIF_STATE) != 0 && (nmlv.uNewState & LVIS_SELECTED) != 0 {
                        let sel = nmlv.iItem as isize;
                        if sel >= 0 {
                            {
                                let mut rename_idx = RENAME_INDEX.lock().unwrap();
                                if *rename_idx != None {
                                    *rename_idx = None;
                                    let edit_hwnd = RENAME_EDIT_HWND.lock().unwrap();
                                    if let Some(edit_val) = *edit_hwnd {
                                        let _ = ShowWindow(HWND(edit_val as *mut std::ffi::c_void), SW_HIDE);
                                    }
                                }
                            }

                            let mut selected = CONFIG_SELECTED.lock().unwrap();
                            *selected = Some(sel as usize);

                            APP_STATE.with(|s| {
                                if let Ok(mut state) = s.try_borrow_mut() {
                                    state.load_configs();
                                    state.config_selected = Some(sel as usize);
                                    state.load_config_by_index(sel as usize);
                                }
                            });

                            crate::ui::CURRENT_MODE.store(3, Ordering::SeqCst);
                            if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                if IsWindow(overlay_hwnd).as_bool() {
                                    let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
                                    keep_window_topmost(overlay_hwnd);
                                    let _ = InvalidateRect(overlay_hwnd, None, TRUE);
                                }
                            }
                        }
                    }
                    return LRESULT(0);
                }
                
                if nmhdr.code == NM_DBLCLK as u32 {
                    if let Ok(instance) = GetModuleHandleW(None) {
                        let _ = start_rename(nmhdr.hwndFrom, HINSTANCE(instance.0));
                    }
                    return LRESULT(0);
                }
            }
            return LRESULT(0);
        }
        
        // =====================================================================
        // WM_GETMINMAXINFO - 限制窗口最小大小
        // =====================================================================
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 420;
            mmi.ptMinTrackSize.y = 460;
            LRESULT(0)
        }

        // =====================================================================
        // WM_COMMAND - 处理子窗口控件的消息
        // =====================================================================
        WM_COMMAND => {
            // 提取控件 ID 和通知码
            let control_id = (wparam.0 & 0xFFFF) as i32;
            let notification_code = (wparam.0 >> 16) as i32;

            // 检查是否是配置窗口的消息
            let is_config_window = {
                let cw = CONFIG_WINDOW.lock().unwrap();
                cw.map_or(false, |h| h == hwnd.0 as isize)
            };

            // 只处理配置窗口的消息
            if is_config_window {
                if notification_code == BN_CLICKED as i32 {
                    match control_id {
                        // 加载配置
                        IDB_LOAD_CONFIG => {
                            let mut target_name = String::new();
                            // 使用代码块尽早释放锁，防止与 APP_STATE 冲突
                            {
                                let selected = CONFIG_SELECTED.lock().unwrap();
                                if let Some(idx) = *selected {
                                    let configs = CONFIGS.lock().unwrap();
                                    if idx < configs.len() {
                                        target_name = configs[idx].0.clone();
                                    }
                                }
                            }

                            if !target_name.is_empty() {
                                println!("加载配置: {}", target_name);
                                // 将界面中选中的配置同步加载到运行时状态。
                                APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.load_configs();
                                        let target_file = format!("{}.json", target_name);
                                        if let Some(state_idx) = state.configs.iter().position(|c| c.0 == target_file) {
                                            state.config_selected = Some(state_idx);
                                            state.load_config_by_index(state_idx);
                                        }
                                    }
                                });

                                // 弹出加载成功的提示框
                                let msg = format!("配置文件 \"{}\" 已成功应用！", target_name);
                                let wide_msg: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
                                let wide_title: Vec<u16> = "加载成功\0".encode_utf16().collect();
                                MessageBoxW(hwnd, PCWSTR(wide_msg.as_ptr()), PCWSTR(wide_title.as_ptr()), MB_OK | MB_ICONINFORMATION);
                            }
                            return LRESULT(0);
                        }

                        // 新建配置
                        IDB_NEW_CONFIG => {
                            let configs_dir = get_config_directory();
                            let mut new_name = String::from("新配置");
                            let mut counter = 1;

                            // 检查是否已存在，递增加编号
                            loop {
                                let configs = CONFIGS.lock().unwrap();
                                let exists = configs.iter().any(|(name, _)| name == &new_name);
                                drop(configs);

                                if !exists {
                                    break;
                                }
                                new_name = format!("新配置{}", counter);
                                counter += 1;
                            }

                            // 新配置写入空数组，确保首次加载时没有残留按键数据。
                            let new_path = configs_dir.join(format!("{}.json", new_name));
                            let default_content = "[]"; 
                            
                            if fs::write(&new_path, default_content).is_ok() {
                                // 刷新列表
                                if let Ok(_instance) = GetModuleHandleW(None) {
                                    let list_hwnd = GetDlgItem(hwnd, IDL_CONFIG_LIST);
                                    if !list_hwnd.is_err() {
                                        let list_hwnd = list_hwnd.unwrap();
                                        
                                        crate::config::load_configs();
                                        
                                        let configs = CONFIGS.lock().unwrap();
                                        if let Some(ui_idx) = configs.iter().position(|(n, _)| n == &new_name) {
                                            *CONFIG_SELECTED.lock().unwrap() = Some(ui_idx);
                                            drop(configs);
                                            
                                            crate::ui::panels::refresh_config_list(list_hwnd, None);
                                            
                                            APP_STATE.with(|s| {
                                                if let Ok(mut state) = s.try_borrow_mut() {
                                                    state.load_configs();
                                                    let target_file = format!("{}.json", new_name);
                                                    if let Some(state_idx) = state.configs.iter().position(|c| c.0 == target_file) {
                                                        state.config_selected = Some(state_idx);
                                                        state.load_config_by_index(state_idx);
                                                    }
                                                    
                                                    if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                                        if IsWindow(overlay_hwnd).as_bool() {
                                                            crate::ui::render::force_redraw(overlay_hwnd, &mut state);
                                                        }
                                                    }
                                                }
                                            });
                                        } else {
                                            drop(configs);
                                            crate::ui::panels::refresh_config_list(list_hwnd, None);
                                        }
                                    }
                                }
                            }
                            return LRESULT(0);
                        }

                        // 复制当前选中的配置
                        IDB_COPY_CONFIG => {
                            // 1. 获取当前选中的配置名称和路径
                            let (source_name, source_path) = {
                                let selected = CONFIG_SELECTED.lock().unwrap();
                                if let Some(idx) = *selected {
                                    let configs = CONFIGS.lock().unwrap();
                                    if idx < configs.len() {
                                        (Some(configs[idx].0.clone()), Some(configs[idx].1.clone()))
                                    } else {
                                        (None, None)
                                    }
                                } else {
                                    (None, None)
                                }
                            };

                            // 如果有选中的文件，则执行复制
                            if let (Some(name), Some(path)) = (source_name, source_path) {
                                let configs_dir = get_config_directory();
                                let mut new_name = format!("{}_副本", name);
                                let mut counter = 1;

                                // 检查重名并递增编号
                                loop {
                                    let configs = CONFIGS.lock().unwrap();
                                    let exists = configs.iter().any(|(n, _)| n == &new_name);
                                    drop(configs);

                                    if !exists {
                                        break;
                                    }
                                    counter += 1;
                                    new_name = format!("{}_副本{}", name, counter);
                                }

                                // 复制文件
                                let new_path = configs_dir.join(format!("{}.json", new_name));
                                if std::fs::copy(&path, &new_path).is_ok() {
                                    // 刷新列表
                                    if let Ok(_instance) = GetModuleHandleW(None) {
                                        let list_hwnd = GetDlgItem(hwnd, IDL_CONFIG_LIST);
                                        if !list_hwnd.is_err() {
                                            refresh_config_list(list_hwnd.unwrap(), None);
                                        }
                                    }
                                }
                            } else {
                                // 没有选中配置时给予提示
                                MessageBoxW(hwnd, w!("请先在列表中选择一个要复制的配置文件。"), w!("提示"), MB_OK | MB_ICONINFORMATION);
                            }
                            return LRESULT(0);
                        }

                        // 删除配置
                        IDB_DELETE_CONFIG => {
                            // 获取要删除的配置信息
                            let (config_name, path_to_delete) = {
                                let mut selected = CONFIG_SELECTED.lock().unwrap();
                                if let Some(idx) = *selected {
                                    let configs = CONFIGS.lock().unwrap();
                                    if idx < configs.len() {
                                        let (name, path) = configs[idx].clone();
                                        *selected = None;
                                        (Some(name), Some(path))
                                    } else {
                                        (None, None)
                                    }
                                } else {
                                    (None, None)
                                }
                            };

                            // 如果没有选中配置，直接返回
                            let Some(config_name) = config_name else {
                                return LRESULT(0);
                            };
                            let Some(path) = path_to_delete else {
                                return LRESULT(0);
                            };

                            // 构建警告消息
                            let warning_title = "确认删除";
                            let warning_title_wide: Vec<u16> = warning_title.encode_utf16().chain(std::iter::once(0)).collect();
                            
                            let warning_msg = format!(
                                "警告：删除操作不可恢复！\n\n\
                                您确定要删除配置文件 \"{}\" 吗？\n\n\
                                点击\"是(Y)\"将永久删除此配置文件。",
                                config_name
                            );
                            let warning_msg_wide: Vec<u16> = warning_msg.encode_utf16().chain(std::iter::once(0)).collect();

                            // 显示确认对话框 (MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2)
                            // MB_YESNO: 显示"是"和"否"按钮
                            // MB_ICONWARNING: 显示警告图标
                            // MB_DEFBUTTON2: 默认焦点在第二个按钮(否)上，更安全
                            let result = MessageBoxW(
                                hwnd,
                                PCWSTR(warning_msg_wide.as_ptr()),
                                PCWSTR(warning_title_wide.as_ptr()),
                                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                            );

                            // 如果用户确认删除 (IDYES = 6)
                            if result == IDYES {
                                let _ = fs::remove_file(&path);

                                // 删除配置后立即清空内存中的按键数据，并刷新悬浮层显示。
                                APP_STATE.with(|s| {
                                    if let Ok(mut state) = s.try_borrow_mut() {
                                        state.config_selected = None;
                                        state.buttons.clear(); // 彻底清空内存里的旧按键
                                        state.load_configs();
                                        
                                        // 强制将"空白状态"重绘到屏幕上
                                        if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                                            if IsWindow(overlay_hwnd).as_bool() {
                                                crate::ui::render::force_redraw(overlay_hwnd, &mut state);
                                            }
                                        }
                                    }
                                });

                                // 刷新左侧配置列表
                                let list_hwnd = GetDlgItem(hwnd, IDL_CONFIG_LIST);
                                if !list_hwnd.is_err() {
                                    refresh_config_list(list_hwnd.unwrap(), None);
                                }
                            }
                            return LRESULT(0);
                        }

                        // 打开文件夹
                        IDB_OPEN_FOLDER => {
                            let configs_dir = get_config_directory();
                            let _ = ensure_configs_dir();
                            let dir_str = configs_dir.to_string_lossy();
                            let dir_wide: Vec<u16> = dir_str.encode_utf16().chain(std::iter::once(0)).collect();
                            // 使用 ShellExecuteW 打开文件夹
                            let _ = ShellExecuteW(
                                None,
                                w!("open"),
                                PCWSTR(dir_wide.as_ptr()),
                                None,
                                None,
                                SW_SHOW,
                            );
                            return LRESULT(0);
                        }

                        // 重命名配置（启动内联编辑）
                        IDB_RENAME_CONFIG => {
                            // 获取 ListBox 句柄
                            let list_hwnd = GetDlgItem(hwnd, IDL_CONFIG_LIST);
                            if let Ok(list_hwnd) = list_hwnd {
                                // 获取模块实例句柄
                                if let Ok(instance) = GetModuleHandleW(None) {
                                    // 调用 start_rename 开始重命名
                                    let _ = start_rename(list_hwnd, HINSTANCE(instance.0));
                                }
                            }
                            return LRESULT(0);
                        }

                        _ => {}
                    }
                }
            }

            LRESULT(0)
        }

        // =====================================================================
        // WM_DESTROY - 窗口销毁时发送
        // =====================================================================
        WM_DESTROY => {
            // 检查是否是配置窗口
            let is_config = {
                let cw = CONFIG_WINDOW.lock().unwrap();
                cw.map_or(false, |h| h == hwnd.0 as isize)
            };

            // 如果是配置窗口，清理相关状态
            if is_config {
                let mut cw = CONFIG_WINDOW.lock().unwrap();
                *cw = None;

                let mut edit_hwnd = RENAME_EDIT_HWND.lock().unwrap();
                *edit_hwnd = None;

                let mut rename_idx = RENAME_INDEX.lock().unwrap();
                *rename_idx = None;

                if crate::ui::CURRENT_MODE.load(Ordering::SeqCst) == 3 {
                    crate::ui::CURRENT_MODE.store(0, Ordering::SeqCst);
                    if let Ok(overlay_hwnd) = FindWindowW(w!("OmniTouch-OverlayWindow"), w!("OmniTouch Overlay")) {
                        if IsWindow(overlay_hwnd).as_bool() { let _ = ShowWindow(overlay_hwnd, SW_HIDE); }
                    }
                }
            }
            LRESULT(0)
        }

        // =====================================================================
        // WM_SETTINGCHANGE - 监听系统主题/深浅模式切换
        // =====================================================================
        WM_SETTINGCHANGE => {
            let _ = crate::ui::base::apply_window_attributes(hwnd);
            let _ = InvalidateRect(hwnd, None, TRUE);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }

        // =====================================================================
        // 默认消息处理
        // =====================================================================
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// =============================================================================
// 悬浮窗口过程
// =============================================================================
/// 悬浮层窗口过程，负责悬浮键盘和运行态覆盖层消息。
pub unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERUPDATE: u32 = 0x0245;

    match message {
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let mut handled = false;
            let mut need_redraw = false;
            
            APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    if state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu {
                        let pointer_id = (wparam.0 & 0xFFFF) as u32;
                        let x = (lparam.0 & 0xFFFF) as i16 as i32;
                        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                        let pt = POINT { x, y };

                        if message == WM_POINTERDOWN {
                            crate::core::event_handler::on_pointer_down(&mut state, pointer_id, pt);
                            need_redraw = true;
                        } else if message == WM_POINTERUPDATE {
                            need_redraw = crate::core::event_handler::on_pointer_update(&mut state, pointer_id, pt);
                        } else if message == WM_POINTERUP {
                            crate::core::event_handler::on_pointer_up(&mut state, pointer_id);
                            need_redraw = true;
                        }
                        handled = true;
                    }
                }
            });

            if handled {
                if need_redraw { let _ = InvalidateRect(hwnd, None, FALSE); }
                return LRESULT(0); 
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_NCHITTEST => {
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let mut hit: isize = HTTRANSPARENT as isize;
            APP_STATE.with(|s| {
                if let Ok(s) = s.try_borrow() {
                    // 命中悬浮键盘区域时直接接管点击，避免输入穿透到底层窗口。
                    if s.osk_visible && PtInRect(&s.osk_rect, pt).as_bool() {
                        hit = HTCLIENT as isize;
                    }
                    // 未命中悬浮键盘时，仅在运行态拦截屏幕上的虚拟按键区域。
                    else if s.mode == ProgramMode::Running {
                        for btn in &s.buttons {
                            if PtInRect(&btn.rect.into(), pt).as_bool() {
                                hit = HTCLIENT as isize;
                                break;
                            }
                        }
                    }
                }
            });
            return LRESULT(hit);
        }
        WM_MOUSEACTIVATE => {
            ensure_mouse_hook();
            return LRESULT(MA_NOACTIVATE as isize);
        }
        // 鼠标按下时交给统一事件处理器更新按键状态。
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    crate::core::event_handler::on_lbutton_down(&mut s, hwnd, pt, false);
                    crate::ui::render::force_redraw(hwnd, &mut s);
                }
            });
            return LRESULT(0);
        }
        // 鼠标移动时处理拖拽、摇杆和滑动触发逻辑。
        WM_MOUSEMOVE => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    let need_redraw = crate::core::event_handler::on_mouse_move(&mut s, pt);
                    if need_redraw {
                        crate::ui::render::force_redraw(hwnd, &mut s);
                    }
                }
            });
            return LRESULT(0);
        }
        // 鼠标抬起时统一收尾并释放按键状态。
        WM_LBUTTONUP => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let _ = ReleaseCapture();
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    crate::core::event_handler::on_lbutton_up(&mut s);
                    crate::ui::render::force_redraw(hwnd, &mut s);
                }
            });
            return LRESULT(0);
        }
        WM_PAINT => {
            let mode = crate::ui::CURRENT_MODE.load(Ordering::SeqCst);
            let mut ps = PAINTSTRUCT::default();
            let _hdc = BeginPaint(hwnd, &mut ps);
            
            match mode {
                0 => {
                    APP_STATE.with(|s| {
                        if let Ok(state) = s.try_borrow_mut() {
                            state.buffer.present(hwnd);
                        }
                    });
                }
                2 | 3 => {
                    APP_STATE.with(|s| {
                        if let Ok(mut state) = s.try_borrow_mut() {
                            let old_mode = state.mode.clone();
                            state.mode = ProgramMode::Running; 
                            crate::ui::render::force_redraw(hwnd, &mut state);
                            state.mode = old_mode;
                        }
                    });
                }
                _ => {}
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == 100 {
                keep_window_topmost(hwnd);
                APP_STATE.with(|s| {
                    if let Ok(state) = s.try_borrow_mut() {
                        if state.mode == ProgramMode::Running {
                            // 仅在启用虚拟手柄时发送状态同步，减少无效开销。
                            if state.use_virtual_gamepad {
                                crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
                            }

                            if let Some(idx) = state.touchpad_active_button {
                                let btn = &state.buttons[idx];
                                if btn.group == 6 && btn.is_pressed && btn.variant == crate::app_state::ButtonVariant::Joystick {
                                    let (jx, jy) = btn.joystick_val;
                                    let sens = btn.sensitivity;
                                    if jx.abs() > 0.05 || jy.abs() > 0.05 {
                                        let speed = 20.0 * sens;
                                        let dx = (jx * speed) as i32;
                                        let dy = (jy * speed) as i32;
                                        if dx != 0 || dy != 0 {
                                            crate::input::handler::simulate_mouse_move_relative(dx, dy);
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
            }
            return LRESULT(0);
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
// =============================================================================
// 编辑窗口过程 (完全还原旧项目逻辑)
// =============================================================================
/// 编辑层窗口过程，负责编辑态下的输入、拖拽与重绘逻辑。
pub unsafe extern "system" fn edit_wndproc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERUPDATE: u32 = 0x0245;

    match message {
        WM_CLOSE => {
            let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
            if !sidebar_handle.is_invalid() {
                let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                if IsWindow(sidebar_hwnd).as_bool() {
                    SendMessageW(sidebar_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    return LRESULT(0);
                }
            }
            return DefWindowProcW(window, message, wparam, lparam);
        }
        WM_MOUSEACTIVATE => {
            ensure_mouse_hook();
            return LRESULT(MA_NOACTIVATE as isize);
        }
        WM_NCHITTEST => {
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let mut hit: isize = HTTRANSPARENT as isize;
            APP_STATE.with(|s| {
                if let Ok(s) = s.try_borrow() {
                    if s.osk_visible && PtInRect(&s.osk_rect, pt).as_bool() {
                        hit = HTCLIENT as isize;
                        return;
                    }

                    let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
                    if !sidebar_handle.is_invalid() {
                        let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                        if IsWindow(sidebar_hwnd).as_bool() {
                            let mut rect = RECT::default();
                            if GetWindowRect(sidebar_hwnd, &mut rect).is_ok() {
                                if PtInRect(&rect, pt).as_bool() {
                                    hit = HTTRANSPARENT as isize;
                                    return;
                                }
                            }
                        }
                    }

                    match s.mode {
                        ProgramMode::Running | ProgramMode::Menu => {
                            for btn in &s.buttons {
                                if PtInRect(&btn.rect.into(), pt).as_bool() {
                                    hit = HTCLIENT as isize;
                                    break;
                                }
                            }
                        }
                        ProgramMode::Editing | ProgramMode::ButtonDetail(_) | ProgramMode::GroupDetail(_) => {
                            hit = HTCLIENT as isize;
                        }
                        _ => {}
                    }
                }
            });
            return LRESULT(hit);
        }
        WM_TIMER => {
            keep_window_topmost(window);
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    if s.osk_visible && s.osk_target_text.is_some() {
                        crate::ui::render::force_redraw(window, &mut s);
                    }
                }
            });
            return LRESULT(0);
        }
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            let mut close = false;
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    if message == WM_POINTERDOWN {
                        crate::core::event_handler::on_lbutton_down(&mut s, window, pt, false);
                    } else if message == WM_POINTERUPDATE {
                        let need_redraw = crate::core::event_handler::on_mouse_move(&mut s, pt);
                        if need_redraw {
                            crate::ui::render::force_redraw(window, &mut s);
                        }
                        return;
                    } else {
                        crate::core::event_handler::on_lbutton_up(&mut s);
                    }

                    if s.mode == ProgramMode::Menu || s.mode == ProgramMode::Paused {
                        close = true;
                        s.emergency_release_all_keys();
                    } else {
                        crate::ui::render::force_redraw(window, &mut s);
                    }
                }
            });

            if close {
                crate::ui::CURRENT_MODE.store(0, std::sync::atomic::Ordering::SeqCst);

                let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
                if !sidebar_handle.is_invalid() {
                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                    if IsWindow(sidebar_hwnd).as_bool() { DestroyWindow(sidebar_hwnd); }
                    RemovePropW(window, w!("SidebarHwnd"));
                }

                let main_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
                if IsWindow(main_hwnd).as_bool() { ShowWindow(main_hwnd, SW_SHOW); }
                DestroyWindow(window);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            let is_double_click = message == WM_LBUTTONDBLCLK;
            let mut close = false;
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    crate::core::event_handler::on_lbutton_down(&mut s, window, pt, is_double_click);
                    if s.mode == ProgramMode::Menu || s.mode == ProgramMode::Paused {
                        close = true;
                        // 【关键修复：退出编辑模式时释放所有按键，防止粘连！】
                        s.emergency_release_all_keys();
                    } else {
                        crate::ui::render::force_redraw(window, &mut s);
                    }
                }
            });
            
            if close {
                crate::ui::CURRENT_MODE.store(0, std::sync::atomic::Ordering::SeqCst);
                
                let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
                if !sidebar_handle.is_invalid() {
                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                    if IsWindow(sidebar_hwnd).as_bool() { DestroyWindow(sidebar_hwnd); }
                    RemovePropW(window, w!("SidebarHwnd"));
                }
 
                let main_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
                if IsWindow(main_hwnd).as_bool() { ShowWindow(main_hwnd, SW_SHOW); }
                DestroyWindow(window);
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    let need_redraw = crate::core::event_handler::on_mouse_move(&mut s, pt);
                    if need_redraw {
                        crate::ui::render::force_redraw(window, &mut s);
                    }
                }
            });
            return LRESULT(0);
        }

        WM_RBUTTONUP => {
            let mut close = false;
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    if s.mode == ProgramMode::Menu || s.mode == ProgramMode::Paused {
                        close = true;
                        // 【关键修复：退出编辑模式时释放所有按键，防止粘连！】
                        s.emergency_release_all_keys();
                    }
                }
            });

            if close {
                crate::ui::CURRENT_MODE.store(0, std::sync::atomic::Ordering::SeqCst);

                let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
                if !sidebar_handle.is_invalid() {
                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                    if IsWindow(sidebar_hwnd).as_bool() { DestroyWindow(sidebar_hwnd); }
                    RemovePropW(window, w!("SidebarHwnd"));
                }

                let main_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
                if IsWindow(main_hwnd).as_bool() { ShowWindow(main_hwnd, SW_SHOW); }
                DestroyWindow(window);
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            if is_synthetic_touch_mouse_message() || is_injected_mouse_message() {
                return LRESULT(0);
            }
            let mut close = false;
            APP_STATE.with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    crate::core::event_handler::on_lbutton_up(&mut s);
                    if s.mode == ProgramMode::Menu || s.mode == ProgramMode::Paused {
                        close = true;
                        // 【关键修复：退出编辑模式时释放所有按键，防止粘连！】
                        s.emergency_release_all_keys();
                    } else {
                        crate::ui::render::force_redraw(window, &mut s);
                    }
                }
            });
            
            if close {
                crate::ui::CURRENT_MODE.store(0, std::sync::atomic::Ordering::SeqCst);
                
                let sidebar_handle = GetPropW(window, w!("SidebarHwnd"));
                if !sidebar_handle.is_invalid() {
                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                    if IsWindow(sidebar_hwnd).as_bool() { DestroyWindow(sidebar_hwnd); }
                    RemovePropW(window, w!("SidebarHwnd"));
                }
 
                let main_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
                if IsWindow(main_hwnd).as_bool() { ShowWindow(main_hwnd, SW_SHOW); }
                DestroyWindow(window);
            }
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            return LRESULT(0);
        }
        WM_DESTROY => {
            let main_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
            if IsWindow(main_hwnd).as_bool() {
                ShowWindow(main_hwnd, SW_SHOW);
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}

/// 侧边栏窗口过程，负责树控件、按钮和属性面板布局。
pub unsafe extern "system" fn sidebar_wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 800;
            mmi.ptMinTrackSize.y = 600;
            return LRESULT(0);
        }
        WM_SIZE => {
            let width = (lparam.0 & 0xFFFF) as i32;
            let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
            let tree_hwnd = GetDlgItem(window, 3002).unwrap_or_default();
            let btn_save = GetDlgItem(window, 5002).unwrap_or_default();
            let btn_exit = GetDlgItem(window, 5003).unwrap_or_default();
            let right_pane_handle = GetPropW(window, w!("RightPane"));
            
            if IsWindow(tree_hwnd).as_bool() {
                let tree_h = height - 70;
                SetWindowPos(tree_hwnd, HWND::default(), 10, 10, 260, tree_h, SWP_NOZORDER);
                let btn_y = height - 50;
                SetWindowPos(btn_save, HWND::default(), 10, btn_y, 80, 40, SWP_NOZORDER);
                SetWindowPos(btn_exit, HWND::default(), 100, btn_y, 80, 40, SWP_NOZORDER);

                if !right_pane_handle.is_invalid() {
                    let right_hwnd = HWND(right_pane_handle.0 as *mut _);
                    SetWindowPos(right_hwnd, HWND::default(), 280, 10, width - 290, height - 20, SWP_NOZORDER);
                }
            }
            return LRESULT(0);
        }
        WM_COMMAND => {
            let ctrl_id = (wparam.0 & 0xFFFF) as i32;
            let edit_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);

            if ctrl_id == 5003 {
                let _ = PostMessageW(window, WM_CLOSE, WPARAM(0), LPARAM(0));
                return LRESULT(0);
            }

            APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    if ctrl_id == 5002 {
                        state.save_to_selected();
                        MessageBoxW(window, w!("配置已保存"), w!("提示"), MB_OK | MB_ICONINFORMATION);
                    }
                }
            });
            return LRESULT(0);
        }
        WM_NOTIFY => {
            let nmhdr = &*(lparam.0 as *const NMHDR);
            if nmhdr.idFrom == 3002 && nmhdr.code == TVN_SELCHANGEDW {
                let nmtv = &*(lparam.0 as *const NMTREEVIEWW);
                let data = nmtv.itemNew.lParam.0 as isize;
                APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        let edit_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
                        if data < 0 { 
                            let btn_idx = (-data - 1) as usize;
                            state.mode = crate::app_state::ProgramMode::ButtonDetail(btn_idx);
                            crate::ui::panels::show_button_property_window(edit_hwnd, btn_idx, &mut state);
                            if IsWindow(edit_hwnd).as_bool() { crate::ui::render::force_redraw(edit_hwnd, &mut state); }
                        } else if data >= 20000 { 
                            let cat_idx = ((data - 20000) / 1000) as usize;
                            let inst_idx = ((data - 20000) % 1000) as usize;
                            
                            let mut groups: std::collections::HashSet<u32> = std::collections::HashSet::new();
                            for btn in &state.buttons {
                                if btn.combo_category == cat_idx as i32 {
                                    groups.insert(btn.group_id);
                                }
                            }
                            let mut group_ids: Vec<u32> = groups.into_iter().collect();
                            group_ids.sort();
                            
                            if inst_idx < group_ids.len() {
                                let gid = group_ids[inst_idx];
                                crate::ui::panels::show_combo_instance_window(edit_hwnd, cat_idx, inst_idx, gid, &mut state);
                            }
                        } else if data >= 1000 && data < 2000 { 
                            let combo_idx = (data - 1000) as usize;
                            crate::ui::panels::show_category_add_window(edit_hwnd, 0, combo_idx, &mut state);
                        } else if data > 0 { 
                            let cat_idx = (data - 1) as usize;
                            if cat_idx < 7 {
                                crate::ui::panels::show_category_add_window(edit_hwnd, 1, cat_idx, &mut state);
                            }
                        } else { 
                            let old_prop = GetPropW(edit_hwnd, w!("PropertyHwnd"));
                            if !old_prop.is_invalid() {
                                DestroyWindow(HWND(old_prop.0 as *mut _));
                                RemovePropW(edit_hwnd, w!("PropertyHwnd"));
                            }
                        }
                    }
                });
            }
            return LRESULT(0);
        }
        WM_CLOSE => {
            let edit_hwnd = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);

            let mut should_close = true;
            let mut should_save = false;
            let mut should_revert = false;

            APP_STATE.with(|s| {
                if let Ok(state) = s.try_borrow() {
                    if state.has_unsaved_changes() {
                        let res = MessageBoxW(
                            window,
                            w!("当前按键配置已被修改，是否保存更改？\n\n选择“是”保存，选择“否”放弃更改。"),
                            w!("保存确认"),
                            MB_YESNOCANCEL | MB_ICONQUESTION
                        );

                        if res == IDYES {
                            should_save = true;
                        } else if res == IDNO {
                            should_revert = true;
                        } else {
                            should_close = false;
                        }
                    }
                }
            });

            if !should_close {
                return LRESULT(0);
            }

            APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    if should_save {
                        state.save_to_selected();
                    } else if should_revert {
                        if let Some(idx) = state.config_selected {
                            state.load_config_by_index(idx);
                        }
                    }
                    state.mode = ProgramMode::Menu;
                    crate::ui::CURRENT_MODE.store(0, std::sync::atomic::Ordering::SeqCst);
                }
            });

            if IsWindow(edit_hwnd).as_bool() {
                let main_hwnd = HWND(GetWindowLongPtrW(edit_hwnd, GWLP_USERDATA) as *mut _);
                if IsWindow(main_hwnd).as_bool() { ShowWindow(main_hwnd, SW_SHOW); }
                RemovePropW(edit_hwnd, w!("SidebarHwnd"));
                DestroyWindow(edit_hwnd);
            }
            DestroyWindow(window);
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            let _ = crate::ui::base::apply_window_attributes(window);
            let _ = InvalidateRect(window, None, TRUE);
            return DefWindowProcW(window, message, wparam, lparam);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}

/// 读取系统主题设置，判断当前是否为深色模式。
pub fn is_dark_mode() -> bool {
    let mut is_dark = false;
    unsafe {
        use windows::Win32::System::Registry::*;
        let mut hkey = HKEY::default();
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;

        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            0,
            KEY_READ,
            &mut hkey
        ).is_ok() {
            if RegQueryValueExW(hkey, w!("AppsUseLightTheme"), None, None, Some(&mut value as *mut _ as *mut u8), Some(&mut size)).is_ok() {
                is_dark = value == 0;
            }
            let _ = RegCloseKey(hkey);
        }
    }
    is_dark
}
