//! 文件作用：提供基础 UI 能力，包括窗口视觉效果、主菜单按钮和侧边栏同步。

use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use crate::app_state::AppState;

/// 存储主菜单按钮句柄，便于在重建布局时统一销毁。
pub static MENU_BUTTONS: Lazy<Mutex<Vec<isize>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// 缓存一份全局字体句柄，避免重复创建造成 GDI 资源泄漏。
pub static MODERN_FONT: Lazy<Mutex<isize>> = Lazy::new(|| {
    unsafe {
        let h_font = CreateFontW(
            -16, 0, 0, 0, 500, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32, CLEARTYPE_QUALITY.0 as u32,
            VARIABLE_PITCH.0 as u32, w!("Microsoft YaHei UI"),
        );
        Mutex::new(h_font.0 as isize)
    }
});

/// 标记映射功能是否处于激活状态。
pub static MAPPING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 记录当前主界面模式，供菜单按钮和窗口流程共享。
pub static CURRENT_MODE: AtomicU32 = AtomicU32::new(0);

/// 为窗口应用深色标题栏、圆角和 Mica 背景等系统视觉效果。
pub unsafe fn apply_window_attributes(hwnd: HWND) -> Result<()> {
    // 读取系统主题，决定标题栏和背景材质的明暗表现。
    let mut is_dark = false;
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
        if RegQueryValueExW(
            hkey,
            w!("AppsUseLightTheme"),
            None,
            None,
            Some(&mut value as *mut _ as *mut u8),
            Some(&mut size)
        ).is_ok() {
            is_dark = value == 0;
        }
        let _ = RegCloseKey(hkey);
    }

    let dark_mode: u32 = if is_dark { 1 } else { 0 };
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &dark_mode as *const u32 as *const std::ffi::c_void,
        std::mem::size_of::<u32>() as u32,
    )?;

    let mut radius = DWM_WINDOW_CORNER_PREFERENCE(2);
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &mut radius as *mut _ as *const std::ffi::c_void,
        std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
    )?;

    let backdrop = DWM_SYSTEMBACKDROP_TYPE(2);
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_SYSTEMBACKDROP_TYPE,
        &backdrop as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
    )?;

    Ok(())
}

/// 在主窗口中按当前状态重建菜单按钮布局。
pub unsafe fn create_menu_buttons(parent: HWND, instance: HMODULE) -> Result<()> {
    {
        let mut buttons = MENU_BUTTONS.lock().unwrap();
        for &btn in buttons.iter() {
            let _ = DestroyWindow(HWND(btn as *mut std::ffi::c_void));
        }
        buttons.clear();
    }

    let mapping_text = if MAPPING_ACTIVE.load(Ordering::SeqCst) {
        w!("暂停映射")
    } else {
        w!("开始映射")
    };
    let menu_items = [
        (mapping_text, 1001),
        (w!("编辑按键"), 1002),
        (w!("选择配置"), 1003),
        (w!("设置"), 1005),
        (w!("退出"), 1004),
    ];

    let btn_width = 280;
    let btn_height = 40;
    let btn_spacing = 12;
    
    let total_height = menu_items.len() as i32 * btn_height 
        + (menu_items.len() as i32 - 1) * btn_spacing;

    let mut rect = RECT::default();
    GetClientRect(parent, &mut rect)?;
    
    let start_x = (rect.right - rect.left - btn_width) / 2;
    let start_y = (rect.bottom - rect.top - total_height) / 2;

    for (i, (label, cmd_id)) in menu_items.iter().enumerate() {
        let x = start_x;
        let y = start_y + i as i32 * (btn_height + btn_spacing);

        let hwnd_button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            WC_BUTTONW,
            *label,
            WS_TABSTOP | WS_VISIBLE | WS_CHILD | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            x, y,
            btn_width, btn_height,
            parent,
            // 这里的 HMENU 参数实际作为子控件 ID 使用。
            HMENU(*cmd_id as isize as *mut std::ffi::c_void),
            HINSTANCE(instance.0),
            None,
        )?;

        apply_modern_font(hwnd_button);

        let mut buttons = MENU_BUTTONS.lock().unwrap();
        buttons.push(hwnd_button.0 as isize);
    }

    Ok(())
}

/// 为指定控件应用缓存的现代字体。
pub unsafe fn apply_modern_font(hwnd: HWND) {
    let font_ptr = *MODERN_FONT.lock().unwrap();
    if font_ptr != 0 {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(font_ptr as usize), LPARAM(1));
    }
}

/// 根据当前状态重建编辑页左侧的 TreeView 内容。
pub unsafe fn sync_sidebar_list(tree_hwnd: HWND, state: &mut AppState) {
    SendMessageW(tree_hwnd, WM_SETREDRAW, WPARAM(0), LPARAM(0));

    SendMessageW(tree_hwnd, TVM_DELETEITEM, WPARAM(0), LPARAM(TVI_ROOT.0 as isize));

    // 根节点：按键分类。
    let mut tvis_g1 = TVINSERTSTRUCTW::default();
    tvis_g1.hParent = TVI_ROOT;
    tvis_g1.hInsertAfter = TVI_LAST;
    tvis_g1.Anonymous.item.mask = TVIF_TEXT | TVIF_STATE;
    tvis_g1.Anonymous.item.stateMask = TVIS_EXPANDED;
    tvis_g1.Anonymous.item.state = TVIS_EXPANDED;
    let mut w_g1: Vec<u16> = "按键分类".encode_utf16().chain(std::iter::once(0)).collect();
    tvis_g1.Anonymous.item.pszText = PWSTR(w_g1.as_mut_ptr());
    let h_g1 = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_g1 as *const _ as isize)).0);

    let sidebar_names = ["普通按钮", "单击保持", "滑动触发", "触控板", "绝对鼠标", "自由按键", "鼠标摇杆"];
    for (i, cat_name) in sidebar_names.iter().enumerate() {
        let mut tvis_cat = TVINSERTSTRUCTW::default();
        tvis_cat.hParent = h_g1;
        tvis_cat.hInsertAfter = TVI_LAST;
        tvis_cat.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM;
        let mut w_cat: Vec<u16> = cat_name.encode_utf16().chain(std::iter::once(0)).collect();
        tvis_cat.Anonymous.item.pszText = PWSTR(w_cat.as_mut_ptr());
        tvis_cat.Anonymous.item.lParam = LPARAM((i as isize) + 1);
        let h_cat = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_cat as *const _ as isize)).0);

        let mut has_child = false;
        for (btn_idx, btn) in state.buttons.iter().enumerate() {
            if btn.combo_category == -1 && btn.group == i as u8 {
                has_child = true;
                let mut w_btn: Vec<u16> = btn.label.encode_utf16().chain(std::iter::once(0)).collect();
                let mut tvis_btn = TVINSERTSTRUCTW::default();
                tvis_btn.hParent = h_cat;
                tvis_btn.hInsertAfter = TVI_LAST;
                tvis_btn.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM;
                tvis_btn.Anonymous.item.pszText = PWSTR(w_btn.as_mut_ptr());
                tvis_btn.Anonymous.item.lParam = LPARAM(-((btn_idx as isize) + 1));
                SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_btn as *const _ as isize));
            }
        }
        if has_child {
            SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_cat.0 as isize));
        }
    }
    SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_g1.0 as isize));

    // 根节点：按键组合。
    let mut tvis_g2 = TVINSERTSTRUCTW::default();
    tvis_g2.hParent = TVI_ROOT;
    tvis_g2.hInsertAfter = TVI_LAST;
    tvis_g2.Anonymous.item.mask = TVIF_TEXT | TVIF_STATE;
    tvis_g2.Anonymous.item.stateMask = TVIS_EXPANDED;
    tvis_g2.Anonymous.item.state = TVIS_EXPANDED;
    let mut w_g2: Vec<u16> = "按键组合".encode_utf16().chain(std::iter::once(0)).collect();
    tvis_g2.Anonymous.item.pszText = PWSTR(w_g2.as_mut_ptr());
    let h_g2 = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_g2 as *const _ as isize)).0);
    
    // 预设组合单独归档，便于与自定义组合区分。
    let mut tvis_preset = TVINSERTSTRUCTW::default();
    tvis_preset.hParent = h_g2;
    tvis_preset.hInsertAfter = TVI_LAST;
    tvis_preset.Anonymous.item.mask = TVIF_TEXT | TVIF_STATE | TVIF_PARAM;
    tvis_preset.Anonymous.item.stateMask = TVIS_EXPANDED;
    tvis_preset.Anonymous.item.state = TVIS_EXPANDED;
    // lParam 给 0，代表它只是一个单纯的文件夹，点击它时右侧面板会清空留白
    tvis_preset.Anonymous.item.lParam = LPARAM(0); 
    let mut w_preset: Vec<u16> = "预设组合".encode_utf16().chain(std::iter::once(0)).collect();
    tvis_preset.Anonymous.item.pszText = PWSTR(w_preset.as_mut_ptr());
    let h_preset = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_preset as *const _ as isize)).0);

    let combo_names = ["WASD 移动", "方向键", "数字键 (1-0)", "Xbox 手柄布局", "全键盘开关", "自定义组合"];
    for (i, combo_name) in combo_names.iter().enumerate() {
        let mut tvis_combo = TVINSERTSTRUCTW::default();
        
        // 前 5 个是预设组合，最后一个自定义组合保持同级展示。
        tvis_combo.hParent = if i < 5 { h_preset } else { h_g2 };
        
        tvis_combo.hInsertAfter = TVI_LAST;
        tvis_combo.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM;
        let mut w_combo: Vec<u16> = combo_name.encode_utf16().chain(std::iter::once(0)).collect();
        tvis_combo.Anonymous.item.pszText = PWSTR(w_combo.as_mut_ptr());
        tvis_combo.Anonymous.item.lParam = LPARAM(1000 + i as isize);
        let h_combo_cat = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_combo as *const _ as isize)).0);

        let mut groups: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
        for (btn_idx, btn) in state.buttons.iter().enumerate() {
            if btn.combo_category == i as i32 {
                groups.entry(btn.group_id).or_default().push(btn_idx);
            }
        }

        let mut group_ids: Vec<u32> = groups.keys().cloned().collect();
        group_ids.sort();

        for (inst_idx, gid) in group_ids.iter().enumerate() {
            let btn_indices = &groups[gid];
            
            let custom_name = btn_indices.first().and_then(|&idx| state.buttons[idx].group_name.clone());
            let inst_label = custom_name.unwrap_or_else(|| format!("组合 {}", inst_idx + 1));
            let mut w_inst: Vec<u16> = inst_label.encode_utf16().chain(std::iter::once(0)).collect();
            let mut tvis_inst = TVINSERTSTRUCTW::default();
            tvis_inst.hParent = h_combo_cat;
            tvis_inst.hInsertAfter = TVI_LAST;
            tvis_inst.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM;
            tvis_inst.Anonymous.item.pszText = PWSTR(w_inst.as_mut_ptr());
            tvis_inst.Anonymous.item.lParam = LPARAM(20000 + (i as isize * 1000) + inst_idx as isize);
            let h_inst = HTREEITEM(SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_inst as *const _ as isize)).0);

            for &btn_idx in btn_indices {
                let btn = &state.buttons[btn_idx];
                let mut w_btn: Vec<u16> = btn.label.encode_utf16().chain(std::iter::once(0)).collect();
                let mut tvis_btn = TVINSERTSTRUCTW::default();
                tvis_btn.hParent = h_inst;
                tvis_btn.hInsertAfter = TVI_LAST;
                tvis_btn.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM;
                tvis_btn.Anonymous.item.pszText = PWSTR(w_btn.as_mut_ptr());
                tvis_btn.Anonymous.item.lParam = LPARAM(-((btn_idx as isize) + 1));
                SendMessageW(tree_hwnd, TVM_INSERTITEMW, WPARAM(0), LPARAM(&tvis_btn as *const _ as isize));
            }
            SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_inst.0 as isize));
        }

        if !group_ids.is_empty() {
            SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_combo_cat.0 as isize));
        }
    }
    
    SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_preset.0 as isize));
    SendMessageW(tree_hwnd, TVM_EXPAND, WPARAM(TVE_EXPAND.0 as usize), LPARAM(h_g2.0 as isize));

    SendMessageW(tree_hwnd, WM_SETREDRAW, WPARAM(1), LPARAM(0));
    InvalidateRect(tree_hwnd, None, TRUE);
}
