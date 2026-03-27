//! 文件作用：构建配置管理窗口、编辑侧栏属性面板以及组合编辑面板。

// 从配置模块导入必要的函数和全局状态
use crate::config::{ensure_configs_dir, load_configs, CONFIGS, CONFIG_SELECTED};

// 从 wndprocs 模块导入子类化过程
use crate::core::wndprocs::edit_subclass_proc;

// 从 UI 模块导入字体和窗口属性函数
use crate::ui::{apply_modern_font, apply_window_attributes, CURRENT_MODE};

// 导入 once_cell 库的 Lazy 组件，用于延迟初始化全局静态变量
use once_cell::sync::Lazy;

// 导入标准库的互斥锁模块，用于线程安全地访问共享数据
use std::sync::Mutex;

// 导入 Windows API 核心类型
use windows::core::*;

// 导入 Windows 基础类型（如 HWND、HMODULE、RECT 等）
use windows::Win32::Foundation::*;

// 导入 GDI 图形相关 API
use windows::Win32::Graphics::Gdi::*;

// 导入通用控件 API
use windows::Win32::UI::Controls::*;

// 导入 Windows 消息和窗口管理 API
use windows::Win32::UI::WindowsAndMessaging::*;

// 导入键盘和鼠标 API（用于 SetFocus）
use windows::Win32::UI::Input::KeyboardAndMouse::*;

// 导入 Windows Shell API（用于 SetWindowSubclass）
use windows::Win32::UI::Shell::*;

// =============================================================================
// ListView 专属常量和结构体
// =============================================================================
const WC_LISTVIEWW: windows::core::PCWSTR = w!("SysListView32");
const LVS_REPORT: u32 = 0x0001;
const LVS_SINGLESEL: u32 = 0x0004;
const LVS_SHOWSELALWAYS: u32 = 0x0008;
const LVS_NOCOLUMNHEADER: u32 = 0x4000;
const LVS_EX_FULLROWSELECT: u32 = 0x00000020;
const LVS_EX_DOUBLEBUFFER: u32 = 0x00010000;

const LVM_FIRST: u32 = 0x1000;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
const LVM_SETITEMW: u32 = LVM_FIRST + 76;
const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
const LVM_GETITEMTEXTW: u32 = LVM_FIRST + 115;
const LVM_GETITEMRECT: u32 = LVM_FIRST + 14;

const LVNI_SELECTED: u32 = 0x0002;
const LVIS_SELECTED: u32 = 0x0002;
const LVIS_FOCUSED: u32 = 0x0001;

const LVIF_TEXT: u32 = 0x00000001;
const LVIF_STATE: u32 = 0x00000008;

#[repr(C)]
struct LVCOLUMNW {
    mask: u32, fmt: i32, cx: i32, pszText: PWSTR, cchTextMax: i32,
    iSubItem: i32, iImage: i32, iOrder: i32, cxMin: i32, cxDefault: i32, cxIdeal: i32,
}

#[repr(C)]
struct LVITEMW {
    mask: u32, iItem: i32, iSubItem: i32, state: u32, stateMask: u32,
    pszText: PWSTR, cchTextMax: i32, iImage: i32, lParam: LPARAM,
    iIndent: i32, iGroupId: i32, cColumns: u32, puColumns: *mut u32,
    piColFmt: *mut i32, iGroup: i32,
}

// =============================================================================
// 全局状态变量
// =============================================================================

/// 配置窗口句柄
/// 用于跟踪配置选择窗口的状态
/// Option<isize>: None 表示窗口未创建，Some(hwnd) 表示窗口已创建
pub static CONFIG_WINDOW: Lazy<Mutex<Option<isize>>> = Lazy::new(|| Mutex::new(None));

/// 当前正在重命名的配置索引
/// 用于内联编辑功能，跟踪用户正在重命名的列表项
/// Option<usize>: None 表示不在编辑模式，Some(index) 表示正在编辑的索引
pub static RENAME_INDEX: Lazy<Mutex<Option<usize>>> = Lazy::new(|| Mutex::new(None));

/// 内联编辑框句柄
/// 用于配置名称的快速编辑
/// Option<isize>: None 表示编辑框未创建，Some(hwnd) 表示编辑框已创建
pub static RENAME_EDIT_HWND: Lazy<Mutex<Option<isize>>> = Lazy::new(|| Mutex::new(None));

/// 重命名提交标志
/// 防止在编辑框销毁后重复执行保存操作
pub static RENAME_COMMITTING: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// 重命名时的原始文件名
/// 用于 ESC 键取消时恢复原文件名
pub static RENAME_ORIGINAL_NAME: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

// =============================================================================
// 控件 ID 常量
// =============================================================================
// 这些 ID 用于标识配置窗口中的控件，在 WM_COMMAND 消息中会被发送回窗口过程

/// 配置列表控件 ID (ListBox)
pub const IDL_CONFIG_LIST: i32 = 2001;

/// 内联编辑框 ID（用于重命名）
pub const IDE_RENAME_OK: i32 = 2008;

/// "加载"按钮 ID
pub const IDB_LOAD_CONFIG: i32 = 2002;

/// "新建"按钮 ID
pub const IDB_NEW_CONFIG: i32 = 2003;

/// "删除"按钮 ID
pub const IDB_DELETE_CONFIG: i32 = 2004;

/// "打开文件夹"按钮 ID
pub const IDB_OPEN_FOLDER: i32 = 2005;

/// "重命名"按钮 ID
pub const IDB_RENAME_CONFIG: i32 = 2007;

/// "复制"按钮 ID
pub const IDB_COPY_CONFIG: i32 = 2009;

// =============================================================================
// 公开函数
// =============================================================================

/// 开始重命名操作
/// 
/// # 参数说明
/// - `list_hwnd`: ListBox 控件的句柄
/// - `instance`: 模块实例句柄
/// 
/// # 实现逻辑
/// 1. 获取当前选中项索引
/// 2. 获取选中项的矩形区域
/// 3. 获取选中项的文本
/// 4. 在该位置创建 Edit 控件
/// 5. 子类化 Edit 控件以拦截键盘消息
/// 6. 保存状态
/// 
/// # 注意事项
/// 此函数调用 Windows API，需要在 unsafe 块中执行
pub unsafe fn start_rename(list_hwnd: HWND, instance: HINSTANCE) -> Result<()> {
    println!("[DEBUG start_rename] 开始执行");
    
    let sel = SendMessageW(list_hwnd, LVM_GETNEXTITEM, WPARAM(-1isize as usize), LPARAM(LVNI_SELECTED as isize)).0 as i32;
    println!("[DEBUG start_rename] 选中项索引: {}", sel);
    if sel < 0 {
        println!("[DEBUG start_rename] 没有选中项");
        return Ok(());
    }

    let mut rect = RECT::default();
    rect.left = 0;
    SendMessageW(list_hwnd, LVM_GETITEMRECT, WPARAM(sel as usize), LPARAM(&mut rect as *mut _ as isize));
    println!("[DEBUG start_rename] 矩形: left={}, top={}, right={}, bottom={}", rect.left, rect.top, rect.right, rect.bottom);

    let mut text_buf = vec![0u16; 260];
    let mut item = LVITEMW {
        mask: LVIF_TEXT, iItem: sel, iSubItem: 0, state: 0, stateMask: 0,
        pszText: PWSTR(text_buf.as_mut_ptr()), cchTextMax: 260,
        iImage: 0, lParam: LPARAM(0), iIndent: 0, iGroupId: 0, cColumns: 0,
        puColumns: std::ptr::null_mut(), piColFmt: std::ptr::null_mut(), iGroup: 0,
    };
    SendMessageW(list_hwnd, LVM_GETITEMTEXTW, WPARAM(sel as usize), LPARAM(&mut item as *mut _ as isize));
    let text = String::from_utf16_lossy(&text_buf);
    println!("[DEBUG start_rename] 原始文本: {}", text);

    // 4. 保存原始文件名（用于 ESC 取消时恢复）
    let original_name = text.trim_matches(char::from(0)).to_string();
    {
        let mut original = RENAME_ORIGINAL_NAME.lock().unwrap();
        *original = Some(original_name.clone());
    }
    println!("[DEBUG start_rename] 保存原始文件名: {}", original_name);

    // 5. 创建 EDIT 控件
    // 父窗口设为 ListBox，这样可以简化坐标计算
    // 关键：移除 WS_BORDER，使用 ES_LEFT 左对齐
    // ES_LEFT = 0x0001, ES_AUTOHSCROLL = 0x0080
    let edit_hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("EDIT"),
        PCWSTR(text_buf.as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000 | 0x0080),  // ES_LEFT | ES_AUTOHSCROLL
        rect.left,       // 直接对齐到项的左边界
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        list_hwnd,  // 父窗口设为 ListBox
        HMENU(IDE_RENAME_OK as isize as *mut _),
        instance,
        None,
    )?;
    println!("[DEBUG start_rename] 创建编辑框成功: {:?}", edit_hwnd.0);

    // 核心：手动设置编辑框的左右内边距为 0
    // EC_LEFTMARGIN = 0x0001, EC_RIGHTMARGIN = 0x0002
    // wParam 使用 EC_LEFTMARGIN | EC_RIGHTMARGIN (0x0003) 同时设置左右边距
    // lParam 低16位是左边距，高16位是右边距，0 表示都设为 0
    SendMessageW(edit_hwnd, EM_SETMARGINS, WPARAM((EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize), LPARAM(0));

    // 设置现代字体
    apply_modern_font(edit_hwnd);

    // 选中所有文字（文件名部分）
    let name_str = String::from_utf16_lossy(&text_buf).trim_matches(char::from(0)).to_string();
    let dot_pos = name_str.rfind('.');
    if let Some(pos) = dot_pos {
        SendMessageW(edit_hwnd, EM_SETSEL, WPARAM(0), LPARAM(pos as isize));
        println!("[DEBUG start_rename] 选中文件名部分到位置: {}", pos);
    } else {
        SendMessageW(edit_hwnd, EM_SETSEL, WPARAM(0), LPARAM(-1));
        println!("[DEBUG start_rename] 选中全部文本");
    }

    // 6. 保存全局状态
    *RENAME_INDEX.lock().unwrap() = Some(sel as usize);
    *RENAME_EDIT_HWND.lock().unwrap() = Some(edit_hwnd.0 as isize);
    println!("[DEBUG start_rename] 保存状态完成: RENAME_INDEX={}, RENAME_EDIT_HWND={}", sel, edit_hwnd.0 as isize);

    // 7. 子类化编辑框以拦截键盘消息和焦点消息
    println!("[DEBUG start_rename] 开始子类化编辑框");
    let _ = SetWindowSubclass(edit_hwnd, Some(edit_subclass_proc), 100, 0);
    println!("[DEBUG start_rename] 子类化完成");

    // 先完成子类化，再设置焦点，确保焦点消息能被正确接管。
    let _ = SetFocus(edit_hwnd);
    println!("[DEBUG start_rename] 设置焦点");

    Ok(())
}

/// 刷新配置列表控件
/// 
/// # 参数说明
/// - `hwnd_list`: 配置列表（ListBox）的句柄
/// - `edit_hwnd`: 可选的内联编辑框句柄，如果有则同时处理编辑框的位置和显示
/// 
/// # 实现逻辑
/// 1. 清空列表
/// 2. 重新加载配置文件列表
/// 3. 将配置名称添加到列表
/// 4. 恢复之前的选中状态
/// 5. 如果提供了编辑框句柄，则处理内联编辑的显示和位置
/// 
/// # 内联编辑说明
/// 当用户点击"重命名"按钮时，会显示一个编辑框覆盖在列表项上
/// 用户可以直接在列表中修改名称，无需打开对话框
/// 
/// # 注意事项
/// - 此函数调用 Windows API，需要在 unsafe 块中执行
/// - 使用 LB_GETITEMRECT 消息获取列表项的位置，用于定位编辑框
pub unsafe fn refresh_config_list(hwnd_list: HWND, edit_hwnd: Option<HWND>) {
    SendMessageW(hwnd_list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    load_configs();

    let items: Vec<String> = {
        let configs = CONFIGS.lock().unwrap();
        configs.iter().map(|(name, _)| name.clone()).collect()
    };
    
    let selected_idx = {
        let selected = CONFIG_SELECTED.lock().unwrap();
        *selected
    };

    for (i, name) in items.iter().enumerate() {
        let mut wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: i as i32,
            iSubItem: 0,
            state: 0, stateMask: 0,
            pszText: PWSTR(wide.as_mut_ptr()),
            cchTextMax: 0, iImage: 0, lParam: LPARAM(0), iIndent: 0, iGroupId: 0,
            cColumns: 0, puColumns: std::ptr::null_mut(), piColFmt: std::ptr::null_mut(), iGroup: 0,
        };
        SendMessageW(hwnd_list, LVM_INSERTITEMW, WPARAM(0), LPARAM(&mut item as *mut _ as isize));
    }

    if let Some(idx) = selected_idx {
        if idx < items.len() {
            let mut item = LVITEMW {
                mask: LVIF_STATE,
                state: LVIS_SELECTED | LVIS_FOCUSED,
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                ..std::mem::zeroed()
            };
            SendMessageW(hwnd_list, LVM_SETITEMSTATE, WPARAM(idx), LPARAM(&mut item as *mut _ as isize));
        }
    }

    if let Some(edit) = edit_hwnd {
        let should_show_edit = { *RENAME_INDEX.lock().unwrap() != None };
        if should_show_edit {
            let rename_data = { *RENAME_INDEX.lock().unwrap() };
            if let Some(idx) = rename_data {
                let name_to_edit = if idx < items.len() { Some(items[idx].clone()) } else { None };
                
                if let Some(name) = name_to_edit {
                    let mut item_rect = RECT::default();
                    item_rect.left = 0;
                    SendMessageW(hwnd_list, LVM_GETITEMRECT, WPARAM(idx), LPARAM((&mut item_rect) as *mut _ as isize));

                    let mut list_rect = RECT::default();
                    GetWindowRect(hwnd_list, &mut list_rect);
                    let parent_hwnd = GetParent(hwnd_list).unwrap_or(hwnd_list);
                    let mut parent_window_rect = RECT::default();
                    GetWindowRect(parent_hwnd, &mut parent_window_rect);

                    let list_x = list_rect.left - parent_window_rect.left;
                    let list_y = list_rect.top - parent_window_rect.top;
                    
                    let edit_x = list_x + item_rect.left;
                    let edit_y = list_y + item_rect.top;

                    let _ = SetWindowPos(edit, HWND::default(), edit_x, edit_y, item_rect.right - item_rect.left, item_rect.bottom - item_rect.top, SWP_NOZORDER);

                    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                    let _ = SetWindowTextW(edit, PCWSTR(name_wide.as_ptr()));
                    
                    let dot_pos = name.rfind('.');
                    if let Some(pos) = dot_pos {
                        let _ = SendMessageW(edit, EM_SETSEL, WPARAM(0), LPARAM(pos as isize));
                    } else {
                        let _ = SendMessageW(edit, EM_SETSEL, WPARAM(0), LPARAM(-1));
                    }
                    let _ = ShowWindow(edit, SW_SHOW);
                    let _ = SendMessageW(edit, WM_SETFOCUS, WPARAM(0), LPARAM(0));
                }
            }
        } else {
            let _ = ShowWindow(edit, SW_HIDE);
        }
    }
}

/// 创建配置选择窗口
/// 
/// # 参数说明
/// - `instance`: 模块实例句柄，用于创建窗口和控件
/// 
/// # 返回值
/// 返回 Result 类型：
/// - Ok(()) : 创建成功
/// - Err(e): 创建失败，返回错误信息
/// 
/// # 窗口结构
/// +----------------------------------+
/// |           选择配置               |  <- 标题栏
/// +----------------------------------+
/// |  请选择配置文件:                 |  <- 静态文本标签
/// +----------------------------------+
/// |  [配置文件列表 - ListBox      ] |  <- 列表控件
/// |                                 |
/// +----------------------------------+
/// | [加载] [新建] [删除]             |  <- 第一行按钮
/// | [重命名] [打开文件夹          ]  |  <- 第二行按钮
/// +----------------------------------+
/// 
/// # 实现逻辑
/// 1. 检查窗口是否已存在，如果存在则聚焦并返回
/// 2. 确保配置目录存在
/// 3. 加载配置文件列表
/// 4. 创建配置窗口
/// 5. 创建窗口控件（标签、列表、按钮）
/// 6. 应用 Windows 11 视觉效果
/// 
/// # 注意事项
/// - 此函数调用 Windows API，需要在 unsafe 块中执行
/// - 使用 OmniTouch-ChildWindow 类创建窗口（需要在主程序中注册）
pub unsafe fn create_config_window(instance: HMODULE) -> Result<()> {
    // 步骤 1: 检查窗口是否已存在
    {
        let config_window = CONFIG_WINDOW.lock().unwrap();
        if let Some(hwnd) = *config_window {
            let _ = SetForegroundWindow(HWND(hwnd as *mut std::ffi::c_void));
            return Ok(());
        }
    }

    // 步骤 2: 确保配置目录存在
    if !ensure_configs_dir() {}

    // 步骤 3: 加载配置文件列表
    load_configs();

    // 步骤 4: 创建配置窗口 (调整尺寸并禁用缩放)
    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);
    let w = 460;
    let h = 470;
    let x = (screen_w - w) / 2;
    let y = (screen_h - h) / 2;

    let config_hwnd = CreateWindowExW(
        WS_EX_TOPMOST,
        w!("OmniTouch-ChildWindow"),
        w!("配置管理"),
        // 配置窗口使用固定对话框样式，避免布局被自由拉伸破坏。
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 
        x, y, w, h,
        None, None, HINSTANCE(instance.0), None,
    )?;

    {
        let mut cw = CONFIG_WINDOW.lock().unwrap();
        *cw = Some(config_hwnd.0 as isize);
    }

    // 应用 Windows 11 视觉效果 (深色模式与 Mica 材质)
    apply_window_attributes(config_hwnd)?;

    // 步骤 6: 创建窗口控件

    // 6.1 标题
    let label = CreateWindowExW(
        WINDOW_EX_STYLE::default(), w!("Static"), w!("请选择按键映射方案:"),
        WS_VISIBLE | WS_CHILD, // 移除居中，改为更现代的左对齐
        25, 20, 400, 25, config_hwnd, HMENU::default(), HINSTANCE(instance.0), None,
    )?;
    apply_modern_font(label);

    // 6.2 列表 (升级为现代的 ListView 控件)
    let list_hwnd = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        WC_LISTVIEWW, None,
        WS_VISIBLE | WS_CHILD | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOCOLUMNHEADER),
        25, 55, 395, 200, config_hwnd, HMENU(IDL_CONFIG_LIST as *mut std::ffi::c_void), HINSTANCE(instance.0), None,
    )?;
    
    SendMessageW(list_hwnd, LVM_SETEXTENDEDLISTVIEWSTYLE, WPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as usize), LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize));
    
    let mut col = LVCOLUMNW { mask: 2, fmt: 0, cx: 375, pszText: PWSTR::null(), cchTextMax: 0, iSubItem: 0, iImage: 0, iOrder: 0, cxMin: 0, cxDefault: 0, cxIdeal: 0 };
    SendMessageW(list_hwnd, LVM_INSERTCOLUMNW, WPARAM(0), LPARAM(&mut col as *mut _ as isize));
    
    apply_modern_font(list_hwnd);
    
    let _ = SetWindowTheme(list_hwnd, w!("Explorer"), None); 

    // 6.3 创建内联编辑框 (隐藏备用)
    let rename_edit = CreateWindowExW(
        WS_EX_CLIENTEDGE, w!("Edit"), None, // 改为无黑框的现代输入框
        WS_VISIBLE | WS_CHILD | WINDOW_STYLE(128u32),
        0, 0, 0, 0, config_hwnd, HMENU(IDE_RENAME_OK as *mut std::ffi::c_void), HINSTANCE(instance.0), None,
    )?;
    apply_modern_font(rename_edit);
    let _ = ShowWindow(rename_edit, SW_HIDE);

    {
        let mut edit_hwnd = RENAME_EDIT_HWND.lock().unwrap();
        *edit_hwnd = Some(rename_edit.0 as isize);
    }

    refresh_config_list(list_hwnd, Some(rename_edit));

    // 6.4 现代按钮网格排版 (通过循环生成，保持代码整洁)
    let btn_y = 275;
    let btn_width = 120;
    let btn_height = 36;
    let btn_spacing = 17;

    // 完美 3x2 居中网格
    let btns = [
        (IDB_LOAD_CONFIG, w!("加载配置"), 25, btn_y),
        (IDB_NEW_CONFIG, w!("新建方案"), 25 + btn_width + btn_spacing, btn_y),
        (IDB_COPY_CONFIG, w!("复制方案"), 25 + (btn_width + btn_spacing) * 2, btn_y),
        (IDB_DELETE_CONFIG, w!("删除"), 25, btn_y + btn_height + btn_spacing),
        (IDB_RENAME_CONFIG, w!("重命名"), 25 + btn_width + btn_spacing, btn_y + btn_height + btn_spacing),
        (IDB_OPEN_FOLDER, w!("打开目录"), 25 + (btn_width + btn_spacing) * 2, btn_y + btn_height + btn_spacing),
    ];

    for (id, text, bx, by) in btns {
        let h_btn = CreateWindowExW(
            WINDOW_EX_STYLE::default(), WC_BUTTONW, text,
            WS_TABSTOP | WS_VISIBLE | WS_CHILD | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            bx, by, btn_width, btn_height, config_hwnd, HMENU(id as *mut std::ffi::c_void), HINSTANCE(instance.0), None,
        )?;
        apply_modern_font(h_btn);
        
        // 统一按钮主题，减少默认 3D 控件观感。
        let _ = SetWindowTheme(h_btn, w!("Explorer"), None);
    }

    let _ = ShowWindow(config_hwnd, SW_SHOW);
    Ok(())
}

/// 创建功能子窗口
/// 
/// # 参数说明
/// - `instance`: 模块实例句柄
/// - `title`: 窗口标题（PCWSTR 类型，宽字符串）
/// 
/// # 返回值
/// 返回 Result 类型：
/// - Ok(()) : 创建成功
/// - Err(e): 创建失败，返回错误信息
/// 
/// # 实现功能
/// 创建一个通用的子窗口，用于显示功能内容
/// 窗口包含一个静态文本控件，显示传入的标题
/// 
/// # 注意事项
/// - 此函数调用 Windows API，需要在 unsafe 块中执行
/// - 当前只是一个框架，后续会添加更多功能
pub unsafe fn create_child_window(instance: HMODULE, title: PCWSTR) -> Result<()> {
    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);
    let w = 420;
    let h = 380;
    let x = (screen_w - w) / 2;
    let y = (screen_h - h) / 2;

    let child_hwnd = CreateWindowExW(
        WS_EX_TOPMOST,
        w!("OmniTouch-ChildWindow"),
        title,                  // 窗口标题
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        x, y, w, h,  // 居中位置
        None,
        None,
        HINSTANCE(instance.0),
        None,
    )?;

    // 应用 Windows 11 视觉效果
    apply_window_attributes(child_hwnd)?;

    // 创建静态文本控件（用于显示标题）
    // SS_CENTER (0x00000001): 文本居中显示
    let content = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("Static"),
        title,
        WS_VISIBLE | WS_CHILD | WINDOW_STYLE(0x00000001u32),  // SS_CENTER
        50, 50,                           // 位置
        500, 300,                         // 尺寸
        child_hwnd,
        HMENU::default(),
        HINSTANCE(instance.0),
        None,
    )?;

    // 应用现代字体
    apply_modern_font(content);

    Ok(())
}

// =============================================================================
// 辅助函数
// =============================================================================

/// 验证文件名是否合法
/// 
/// # 参数说明
/// - `name`: 要验证的文件名
/// 
/// # 返回值
/// - true: 文件名合法
/// - false: 文件名包含非法字符
/// 
/// # 禁止的字符
/// Windows 文件系统不允许以下字符出现在文件名中：
/// - \ / : * ? " < > |
/// 此外，文件名不能为空，也不能为 "CON", "PRN", "AUX", "NUL" 等保留名称
pub fn is_valid_filename(name: &str) -> bool {
    // 检查是否为空
    if name.is_empty() {
        return false;
    }

    // 检查长度（Windows 最大255个字符）
    if name.len() > 255 {
        return false;
    }

    // 检查是否包含非法字符
    let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    for c in invalid_chars {
        if name.contains(c) {
            return false;
        }
    }

    // 检查保留名称（不区分大小写）
    let reserved_names = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", 
                          "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", 
                          "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    let name_upper = name.to_uppercase();
    let name_without_ext = name_upper.split('.').next().unwrap_or("");
    if reserved_names.contains(&name_without_ext) {
        return false;
    }

    // 检查是否为设备名
    if name_upper.starts_with("CON.") || 
       name_upper.starts_with("PRN.") || 
       name_upper.starts_with("AUX.") || 
       name_upper.starts_with("NUL.") {
        return false;
    }

    true
}

/// 获取无效字符列表的描述
/// 
/// # 返回值
/// 返回包含无效字符的描述字符串
#[allow(dead_code)]
pub fn get_invalid_chars_description() -> String {
    String::from("文件名不能包含以下字符: \\ / : * ? \" < > |")
}

// =============================================================================
// 编辑模式窗口创建
// =============================================================================

/// 进入编辑模式，创建编辑窗口和侧边栏窗口
/// 
/// # 参数说明
/// - `main_hwnd`: 主窗口句柄
/// - `instance`: 模块实例句柄
/// - `edit_wndproc`: 编辑窗口过程函数
/// - `sidebar_wndproc`: 侧边栏窗口过程函数
/// 
/// # 实现逻辑
/// 1. 注册 Edit 窗口类并创建全屏 edit_hwnd
/// 2. 注册 Sidebar 窗口类并创建独立窗口 sidebar_hwnd
/// 3. 创建 SysTreeView32 控件
/// 4. 设置各种关联并 ShowWindow
/// 5. 切换 AppState 模式并强制重绘
pub unsafe fn enter_edit_mode(
    main_hwnd: HWND,
    instance: HINSTANCE,
    edit_wndproc: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
    sidebar_wndproc: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
) {
    use std::sync::atomic::Ordering;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Controls::*;
    
    CURRENT_MODE.store(1, Ordering::SeqCst);
    let _ = ShowWindow(main_hwnd, SW_HIDE);

    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);
    
    let window_class = w!("OmniTouch-EditWindow");
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hInstance: instance,
        lpszClassName: window_class,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: edit_wndproc,
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc);

    let edit_hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_APPWINDOW | WS_EX_NOACTIVATE,
        window_class,
        w!("OmniTouch-Edit"),
        WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
        0, 0, screen_w, screen_h,
        None, None, instance, None,
    ).unwrap();
    let _ = SetWindowPos(edit_hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    SetWindowLongPtrW(edit_hwnd, GWLP_USERDATA, main_hwnd.0 as isize);

    let sidebar_class = w!("OmniTouch-Sidebar");
    let wc_sidebar = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hInstance: instance,
        lpszClassName: sidebar_class,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: sidebar_wndproc,
        hbrBackground: HBRUSH(COLOR_WINDOW.0 as *mut _),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc_sidebar);

    let w = 800;
    let h = 600;
    let x = (screen_w - w) / 2;
    let y = (screen_h - h) / 2;

    let sidebar_hwnd = CreateWindowExW(
        WS_EX_APPWINDOW | WS_EX_TOPMOST, sidebar_class, w!("编辑菜单"),
        WS_OVERLAPPEDWINDOW, x, y, w, h,
        None, None, instance, None,
    ).unwrap();
    let _ = SetWindowPos(sidebar_hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);

    SetWindowLongPtrW(sidebar_hwnd, GWLP_USERDATA, edit_hwnd.0 as isize);
    SetPropW(edit_hwnd, w!("SidebarHwnd"), HANDLE(sidebar_hwnd.0 as _));
    apply_window_attributes(sidebar_hwnd).unwrap_or(());

    let tree_hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(), w!("SysTreeView32"), None,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_TRACKSELECT | TVS_LINESATROOT) as u32),
        20, 20, 320, 600,
        sidebar_hwnd, HMENU(3002 as *mut _), instance, None,
    ).unwrap();

    let _ = SetWindowTheme(tree_hwnd, w!("Explorer"), None);
    SendMessageW(tree_hwnd, TVM_SETEXTENDEDSTYLE, WPARAM(0x0004), LPARAM(0x0004));
    SendMessageW(tree_hwnd, TVM_SETITEMHEIGHT, WPARAM(55), LPARAM(0));
    apply_modern_font(tree_hwnd);

    let btn_y = 650;
    let btn_save = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("保存"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 20, btn_y, 120, 50, sidebar_hwnd, HMENU(5002 as *mut _), instance, None).unwrap();
    let btn_exit = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("退出"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 160, btn_y, 120, 50, sidebar_hwnd, HMENU(5003 as *mut _), instance, None).unwrap();
    apply_modern_font(btn_save); apply_modern_font(btn_exit);

    let right_pane = CreateWindowExW(
        WINDOW_EX_STYLE::default(), w!("Static"), None,
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        360, 20, 490, 700,
        sidebar_hwnd, HMENU(6001 as *mut _), instance, None,
    ).unwrap();
    SetPropW(sidebar_hwnd, w!("RightPane"), HANDLE(right_pane.0 as _));

    crate::core::wndprocs::APP_STATE.with(|s| {
        if let Ok(mut state) = s.try_borrow_mut() {
            crate::core::event_handler::close_all_osk(&mut state);
            crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
        }
    });

    let _ = ShowWindow(sidebar_hwnd, SW_SHOW);
    SetTimer(edit_hwnd, 1, 500, None);

    crate::core::wndprocs::init_edit_state(edit_hwnd);
}

// =============================================================================
// 原生按键属性面板 (Property Window)
// =============================================================================

/// 处理属性面板输入框中的回车键，避免误触发默认行为。
unsafe extern "system" fn edit_enter_subclass_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize
) -> LRESULT {
    if msg == WM_KEYDOWN {
        if wparam.0 as u16 == VK_RETURN.0 {
            if let Ok(parent) = GetParent(hwnd) {
                if !parent.is_invalid() {
                    let _ = SetFocus(parent);
                }
            }
            return LRESULT(0);
        }
    } else if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(edit_enter_subclass_proc), 0);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// 在树控件中按 `lParam` 递归查找目标节点。
unsafe fn find_tree_item(tree_hwnd: HWND, item: isize, target_lparam: isize) -> isize {
    let mut current = item;
    while current != 0 {
        let mut tvi = TVITEMW::default();
        tvi.mask = TVIF_PARAM;
        tvi.hItem = HTREEITEM(current as _);
        
        SendMessageW(tree_hwnd, TVM_GETITEMW, WPARAM(0), LPARAM(&mut tvi as *mut _ as isize));
        if tvi.lParam.0 == target_lparam {
            return current;
        }
        
        let child = SendMessageW(tree_hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_CHILD as usize), LPARAM(current)).0;
        if child != 0 {
            let found = find_tree_item(tree_hwnd, child, target_lparam);
            if found != 0 { return found; }
        }
        
        current = SendMessageW(tree_hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_NEXT as usize), LPARAM(current)).0;
    }
    0
}

/// 根据节点 `lParam` 选中编辑页左侧树控件中的对应项。
pub unsafe fn select_tree_item_by_lparam(edit_hwnd: HWND, target_lparam: isize) {
    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
    if sidebar_handle.is_invalid() { return; }
    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
    let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
    if tree_hwnd.is_invalid() { return; }

    let root = SendMessageW(tree_hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_ROOT as usize), LPARAM(0)).0;
    if root != 0 {
        let found = find_tree_item(tree_hwnd, root, target_lparam);
        if found != 0 {
            SendMessageW(tree_hwnd, TVM_SELECTITEM, WPARAM(TVGN_CARET as usize), LPARAM(found));
        }
    }
}

/// 在右侧属性区展示指定按键的编辑面板。
pub unsafe fn show_button_property_window(edit_hwnd: HWND, btn_idx: usize, state: &mut crate::app_state::AppState) {
    let instance = HINSTANCE(GetWindowLongPtrW(edit_hwnd, GWLP_HINSTANCE) as *mut _);
    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
    if sidebar_handle.is_invalid() { return; }
    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
    let right_pane = HWND(GetPropW(sidebar_hwnd, w!("RightPane")).0 as *mut _);
    if right_pane.0.is_null() { return; }

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(0), LPARAM(0));

    let old_handle = GetPropW(edit_hwnd, w!("PropertyHwnd"));
    if !old_handle.is_invalid() { DestroyWindow(HWND(old_handle.0 as *mut _)); }

    let class_name = w!("OmniTouch-Property");
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hInstance: instance, lpszClassName: class_name,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(property_wndproc), hbrBackground: HBRUSH(COLOR_WINDOW.0 as *mut _),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc);

    let mut rect = RECT::default();
    GetClientRect(right_pane, &mut rect);

    let prop_hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(), class_name, None,
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_VSCROLL,
        0, 0, rect.right, rect.bottom, right_pane, None, instance, None,
    ).unwrap();

    SetWindowLongPtrW(prop_hwnd, GWLP_USERDATA, btn_idx as isize);
    SetPropW(prop_hwnd, w!("EditHwnd"), HANDLE(edit_hwnd.0 as _));
    SetPropW(edit_hwnd, w!("PropertyHwnd"), HANDLE(prop_hwnd.0 as _));

    let btn = &state.buttons[btn_idx];
    let mut y = 20;

    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("按键名称:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let edit_name = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), 120, y, 250, 25, prop_hwnd, HMENU(4001 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_name, Some(edit_enter_subclass_proc), 0, 0);
    let wide_label: Vec<u16> = btn.label.encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_name, PCWSTR(wide_label.as_ptr()));

    y += 60;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("映射按键:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    
    let mut key_name = format!("VK (0x{:X})", btn.key_code);
    for cat in &state.key_categories {
        for k in &cat.keys { if k.vk.0 == btn.key_code { key_name = k.label.to_string(); break; } }
    }
    let wide_key: Vec<u16> = key_name.encode_utf16().chain(std::iter::once(0)).collect();

    let btn_key = CreateWindowExW(
        WINDOW_EX_STYLE::default(), w!("Button"), PCWSTR(wide_key.as_ptr()), 
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 
        120, y - 5, 250, 30, prop_hwnd, HMENU(4002 as *mut _), instance, None
    ).unwrap();

    if btn.variant == crate::app_state::ButtonVariant::OSKToggle {
        let _ = EnableWindow(btn_key, BOOL(0));
    }

    // 统一采用“标题 + 滑块 + 输入框”的横向布局。
    y += 70;
    let w_val = btn.rect.right - btn.rect.left;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("宽度:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let track_w = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 110, y - 5, 210, 30, prop_hwnd, HMENU(4009 as *mut _), instance, None).unwrap();
    SendMessageW(track_w, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(10, 1000) as isize));
    SendMessageW(track_w, TBM_SETPOS, WPARAM(1), LPARAM(w_val as isize));
    let edit_w = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 330, y - 5, 40, 25, prop_hwnd, HMENU(4007 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_w, Some(edit_enter_subclass_proc), 0, 0);
    let wide_w: Vec<u16> = format!("{}", w_val).encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_w, PCWSTR(wide_w.as_ptr()));

    y += 60;
    let h_val = btn.rect.bottom - btn.rect.top;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("高度:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let track_h = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 110, y - 5, 210, 30, prop_hwnd, HMENU(4010 as *mut _), instance, None).unwrap();
    SendMessageW(track_h, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(10, 1000) as isize));
    SendMessageW(track_h, TBM_SETPOS, WPARAM(1), LPARAM(h_val as isize));
    let edit_h = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 330, y - 5, 40, 25, prop_hwnd, HMENU(4008 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_h, Some(edit_enter_subclass_proc), 0, 0);
    let wide_h: Vec<u16> = format!("{}", h_val).encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_h, PCWSTR(wide_h.as_ptr()));

    y += 60;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("透明度:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let trackbar = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 110, y - 5, 210, 30, prop_hwnd, HMENU(4003 as *mut _), instance, None).unwrap();
    SendMessageW(trackbar, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(0, 255) as isize));
    SendMessageW(trackbar, TBM_SETPOS, WPARAM(1), LPARAM(btn.opacity as isize));
    let edit_op = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 330, y - 5, 40, 25, prop_hwnd, HMENU(4011 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_op, Some(edit_enter_subclass_proc), 0, 0);
    let wide_op: Vec<u16> = format!("{}", btn.opacity).encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_op, PCWSTR(wide_op.as_ptr()));

    // 透明度下方补充一行说明文字，避免用户误解编辑态显示效果。
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("* 透明按键在编辑模式下依旧可见"), WS_CHILD | WS_VISIBLE, 115, y + 25, 250, 20, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);

    // 为提示文字预留额外垂直空间。
    y += 75;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("圆角大小:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let track_rad = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 110, y - 5, 210, 30, prop_hwnd, HMENU(4014 as *mut _), instance, None).unwrap();
    SendMessageW(track_rad, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(0, 500) as isize));
    SendMessageW(track_rad, TBM_SETPOS, WPARAM(1), LPARAM(btn.corner_radius as isize));
    let edit_rad = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 330, y - 5, 40, 25, prop_hwnd, HMENU(4013 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_rad, Some(edit_enter_subclass_proc), 0, 0);
    let wide_rad: Vec<u16> = format!("{}", btn.corner_radius).encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_rad, PCWSTR(wide_rad.as_ptr()));

    y += 60;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("按键类型:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let combo = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("ComboBox"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32), 120, y - 5, 250, 200, prop_hwnd, HMENU(4004 as *mut _), instance, None).unwrap();
    let groups = ["普通按钮", "单击保持", "滑动触发", "触控板", "绝对鼠标", "自由按键", "鼠标摇杆"];
    for item in groups {
        let wide_v: Vec<u16> = item.encode_utf16().chain(std::iter::once(0)).collect();
        SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(wide_v.as_ptr() as isize));
    }
    SendMessageW(combo, CB_SETCURSEL, WPARAM(btn.group as usize), LPARAM(0));

    y += 60;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("灵敏度:"), WS_CHILD | WS_VISIBLE, 30, y, 80, 25, prop_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let track_sens = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 110, y - 5, 210, 30, prop_hwnd, HMENU(4016 as *mut _), instance, None).unwrap();
    // 用放大后的整数范围换取更平滑的灵敏度拖动体验。
    SendMessageW(track_sens, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(10, 1000) as isize));
    SendMessageW(track_sens, TBM_SETPOS, WPARAM(1), LPARAM((btn.sensitivity * 100.0) as isize));
    let edit_sens = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 330, y - 5, 40, 25, prop_hwnd, HMENU(4015 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_sens, Some(edit_enter_subclass_proc), 0, 0);
    let wide_sens: Vec<u16> = format!("{:.1}", btn.sensitivity).encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_sens, PCWSTR(wide_sens.as_ptr()));
    let is_mouse_type = btn.group == 3 || btn.group == 6;
    let _ = EnableWindow(track_sens, BOOL(if is_mouse_type { 1 } else { 0 }));
    let _ = EnableWindow(edit_sens, BOOL(if is_mouse_type { 1 } else { 0 }));

    y += 80;
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("复制此按键"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 30, y, 160, 45, prop_hwnd, HMENU(4012 as *mut _), instance, None);
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("删除此按键"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 210, y, 160, 45, prop_hwnd, HMENU(4005 as *mut _), instance, None);

    let total_height = y + 80;
    SetPropW(prop_hwnd, w!("ContentHeight"), HANDLE(total_height as isize as *mut _));
    SetPropW(prop_hwnd, w!("ScrollPos"), HANDLE(0 as isize as *mut _));

    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: total_height as i32,
        nPage: rect.bottom as u32,
        nPos: 0,
        nTrackPos: 0,
    };
    let _ = SetScrollInfo(prop_hwnd, SB_VERT, &si, TRUE);

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(1), LPARAM(0));
    InvalidateRect(right_pane, None, TRUE);
    let _ = UpdateWindow(right_pane); // 强制父容器彻底擦除旧画面
    
    // 触发非客户区重算，确保滚动条和边框立即可见。
    let _ = SetWindowPos(
        prop_hwnd,
        HWND::default(),
        0, 0, 0, 0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );

    InvalidateRect(prop_hwnd, None, TRUE); 
    let _ = UpdateWindow(prop_hwnd); // 强制面板容器自身重绘

    // 再遍历所有子控件，应用字体并触发它们的强制同步重绘
    let _ = EnumChildWindows(prop_hwnd, Some(apply_font_to_child), LPARAM(0));
    let _ = SetFocus(edit_name);
    
    select_tree_item_by_lparam(edit_hwnd, -((btn_idx as isize) + 1));
}

/// 为面板内所有子控件统一套用现代字体。
unsafe extern "system" fn apply_font_to_child(hwnd: HWND, _: LPARAM) -> BOOL {
    crate::ui::apply_modern_font(hwnd);
    
    // 父窗口冻结绘制期间创建的控件可能缺失边框，这里强制补画非客户区。
    let _ = SetWindowPos(
        hwnd,
        HWND::default(),
        0, 0, 0, 0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    
    // 同步刷新控件内容，避免首次显示残影。
    InvalidateRect(hwnd, None, TRUE);
    let _ = UpdateWindow(hwnd);
    
    TRUE
}

/// 组合两个 16 位值，生成滚动消息需要的 `LPARAM`。
const fn MAKELONG(a: u16, b: u16) -> u32 { ((b as u32) << 16) | (a as u32) }

const TBM_GETPOS: u32 = 1024;

/// 属性面板窗口过程，处理滑块、输入框和按钮交互。
unsafe extern "system" fn property_wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_SIZE => {
            let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
            let content_handle = GetPropW(window, w!("ContentHeight"));
            if !content_handle.is_invalid() {
                let content_height = content_handle.0 as i32;
                let si = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE,
                    nMin: 0,
                    nMax: content_height,
                    nPage: height as u32,
                    ..Default::default()
                };
                let _ = SetScrollInfo(window, SB_VERT, &si, TRUE);
                
                let mut si_pos = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_POS,
                    ..Default::default()
                };
                let _ = GetScrollInfo(window, SB_VERT, &mut si_pos);
                let current_pos = si_pos.nPos;
                let max_scroll = (content_height - height + 1).max(0);
                
                if current_pos > max_scroll {
                    let dy = current_pos - max_scroll;
                    let _ = ScrollWindowEx(window, 0, dy, None, None, None, None, SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE);
                    si_pos.nPos = max_scroll;
                    let _ = SetScrollInfo(window, SB_VERT, &si_pos, TRUE);
                    let _ = SetPropW(window, w!("ScrollPos"), HANDLE(max_scroll as isize as *mut _));
                }
            }
            return LRESULT(0);
        }

        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            let _ = GetScrollInfo(window, SB_VERT, &mut si);
            
            let current_pos = si.nPos;
            let mut new_pos = current_pos - (delta / 120) * 40;
            
            let max_scroll = (si.nMax - si.nPage as i32 + 1).max(0);
            new_pos = new_pos.clamp(0, max_scroll);
            
            if new_pos != current_pos {
                let dy = current_pos - new_pos;
                let _ = ScrollWindowEx(window, 0, dy, None, None, None, None, SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE);
                
                si.fMask = SIF_POS;
                si.nPos = new_pos;
                let _ = SetScrollInfo(window, SB_VERT, &si, TRUE);
                let _ = SetPropW(window, w!("ScrollPos"), HANDLE(new_pos as isize as *mut _));
                let _ = RedrawWindow(window, None, None, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW);
            }
            return LRESULT(0);
        }

        WM_VSCROLL => {
            let mut si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            let _ = GetScrollInfo(window, SB_VERT, &mut si);
            
            let current_pos = si.nPos;
            let mut new_pos = current_pos;
            
            let request = (wparam.0 & 0xFFFF) as u32;
            match request {
                0 => new_pos -= 30,
                1 => new_pos += 30,
                2 => new_pos -= si.nPage as i32,
                3 => new_pos += si.nPage as i32,
                4 | 5 => {
                    new_pos = ((wparam.0 >> 16) & 0xFFFF) as i32;
                    if new_pos == 0 {
                        let mut si_track = SCROLLINFO {
                            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_TRACKPOS,
                            ..Default::default()
                        };
                        let _ = GetScrollInfo(window, SB_VERT, &mut si_track);
                        new_pos = si_track.nTrackPos;
                    }
                }
                6 => new_pos = si.nMin,
                7 => new_pos = si.nMax,
                _ => {}
            }
            
            let max_scroll = (si.nMax - si.nPage as i32 + 1).max(0);
            new_pos = new_pos.clamp(0, max_scroll);
            
            if new_pos != current_pos {
                let dy = current_pos - new_pos;
                let _ = ScrollWindowEx(window, 0, dy, None, None, None, None, SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE);
                
                si.fMask = SIF_POS;
                si.nPos = new_pos;
                let _ = SetScrollInfo(window, SB_VERT, &si, TRUE);
                let _ = SetPropW(window, w!("ScrollPos"), HANDLE(new_pos as isize as *mut _));
                let _ = RedrawWindow(window, None, None, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW);
            }
            return LRESULT(0);
        }
        
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xFFFF) as i32;
            let notify_code = (wparam.0 >> 16) as u32;
            let btn_idx = GetWindowLongPtrW(window, GWLP_USERDATA) as usize;
            let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);

            crate::core::wndprocs::APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    let mut needs_redraw = false;

                    if control_id == 4001 || control_id == 4007 || control_id == 4008 || control_id == 4011 || control_id == 4013 || control_id == 4015 {
                        if notify_code == EN_SETFOCUS {
                            if state.use_system_osk {
                                let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".to_string());
                                let target = format!("{}\\sysnative\\osk.exe", windir);
                                let target = if std::path::Path::new(&target).exists() { target } else { format!("{}\\System32\\osk.exe", windir) };
                                let mut wide_target: Vec<u16> = target.encode_utf16().collect(); wide_target.push(0);
                                let _ = ShellExecuteW(None, w!("open"), PCWSTR(wide_target.as_ptr()), None, None, SW_SHOW);
                            } else {
                                state.osk_visible = true;
                                needs_redraw = true;
                                let _ = SetWindowPos(edit_hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                            }
                        } else if notify_code == EN_KILLFOCUS {
                            if !state.use_system_osk {
                                state.osk_visible = false;
                                needs_redraw = true;
                            }
                        }
                    }

                    if control_id == 4001 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                        GetWindowTextW(h_edit, &mut buf);
                        if let Ok(new_name) = String::from_utf16(&buf[..len as usize]) {
                            state.buttons[btn_idx].label = new_name;
                            needs_redraw = true;

                            let h_edit_main = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                            if IsWindow(h_edit_main).as_bool() {
                                let sidebar_handle = GetPropW(h_edit_main, w!("SidebarHwnd"));
                                if !sidebar_handle.is_invalid() {
                                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                    let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                    if !tree_hwnd.is_invalid() {
                                        crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                    }
                                }
                            }
                        }
                    }
                    else if control_id == 4002 && notify_code == BN_CLICKED as u32 {
                        let h_btn = HWND(lparam.0 as *mut _);
                        let mut rect = RECT::default();
                        GetWindowRect(h_btn, &mut rect);

                        let h_menu = CreatePopupMenu().unwrap_or_default();
                        
                        for cat in &state.key_categories {
                            let h_sub = CreatePopupMenu().unwrap_or_default();
                            for k in &cat.keys {
                                let label = k.label.to_string();
                                let wide_label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                                let menu_id = 10000 + k.vk.0 as usize;
                                let _ = AppendMenuW(h_sub, MF_STRING, menu_id, PCWSTR(wide_label.as_ptr()));
                            }
                            let cat_name = cat.name.to_string();
                            let wide_cat: Vec<u16> = cat_name.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = AppendMenuW(h_menu, MF_POPUP | MF_STRING, h_sub.0 as usize, PCWSTR(wide_cat.as_ptr()));
                        }

                        let selected_id = TrackPopupMenu(
                            h_menu,
                            TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN,
                            rect.left,
                            rect.bottom,
                            0,
                            window,
                            None,
                        ).0 as usize;

                        let _ = DestroyMenu(h_menu);

                        if selected_id >= 10000 {
                            let vk_code = (selected_id - 10000) as u16;
                            
                            let mut new_label = String::new();
                            for cat in &state.key_categories {
                                for k in &cat.keys {
                                    if k.vk.0 == vk_code {
                                        new_label = k.label.to_string();
                                        break;
                                    }
                                }
                            }
                            
                            let btn = &mut state.buttons[btn_idx];
                            if btn.key_code != vk_code {
                                btn.key_code = vk_code;
                                
                                let display_name = if new_label.is_empty() { format!("VK (0x{:X})", vk_code) } else { new_label.clone() };
                                let wide_display: Vec<u16> = display_name.encode_utf16().chain(std::iter::once(0)).collect();
                                SetWindowTextW(h_btn, PCWSTR(wide_display.as_ptr()));

                                if !new_label.is_empty() {
                                    btn.label = new_label.clone();
                                    let h_edit_name = GetDlgItem(window, 4001).unwrap_or_default();
                                    if !h_edit_name.is_invalid() {
                                        let wide_label: Vec<u16> = new_label.encode_utf16().chain(std::iter::once(0)).collect();
                                        SetWindowTextW(h_edit_name, PCWSTR(wide_label.as_ptr()));
                                    }
                                }
                                needs_redraw = true;
                                
                                let h_edit_main = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                                if IsWindow(h_edit_main).as_bool() {
                                    let sidebar_handle = GetPropW(h_edit_main, w!("SidebarHwnd"));
                                    if !sidebar_handle.is_invalid() {
                                        let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                        let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                        if !tree_hwnd.is_invalid() {
                                            crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    else if control_id == 4004 && notify_code == CBN_SELCHANGE as u32 {
                        let h_combo = HWND(lparam.0 as *mut _);
                        let sel = SendMessageW(h_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as isize;
                        
                        if sel >= 0 {
                            let btn = &mut state.buttons[btn_idx];
                            if btn.group != sel as u8 {
                                btn.group = sel as u8;
                                needs_redraw = true;

                                let is_mouse_type = sel == 3 || sel == 6;
                                let h_track = GetDlgItem(window, 4016).unwrap_or_default();
                                let h_edit = GetDlgItem(window, 4015).unwrap_or_default();
                                if !h_track.is_invalid() {
                                    let _ = EnableWindow(h_track, BOOL(if is_mouse_type { 1 } else { 0 }));
                                }
                                if !h_edit.is_invalid() {
                                    let _ = EnableWindow(h_edit, BOOL(if is_mouse_type { 1 } else { 0 }));
                                }
                                
                                let h_edit = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                                if IsWindow(h_edit).as_bool() {
                                    let sidebar_handle = GetPropW(h_edit, w!("SidebarHwnd"));
                                    if !sidebar_handle.is_invalid() {
                                        let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                        let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                        if !tree_hwnd.is_invalid() {
                                            crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    else if control_id == 4005 && notify_code == BN_CLICKED as u32 {
                        state.buttons.remove(btn_idx);
                        state.mode = crate::app_state::ProgramMode::Editing;
                        if IsWindow(edit_hwnd).as_bool() {
                            crate::ui::render::force_redraw(edit_hwnd, &mut state);
                            
                            let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                            if !sidebar_handle.is_invalid() {
                                let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                if !tree_hwnd.is_invalid() {
                                    crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                }
                            }
                        }
                        DestroyWindow(window);
                        return;
                    }
                    else if control_id == 4012 && notify_code == BN_CLICKED as u32 {
                        let mut new_btn = state.buttons[btn_idx].clone();
                        new_btn.rect.left += 30;
                        new_btn.rect.right += 30;
                        new_btn.rect.top += 30;
                        new_btn.rect.bottom += 30;
                        new_btn.label = format!("{}_副本", new_btn.label);
                        
                        state.buttons.push(new_btn);
                        
                        if IsWindow(edit_hwnd).as_bool() {
                            crate::ui::render::force_redraw(edit_hwnd, &mut state);
                            let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                            if !sidebar_handle.is_invalid() {
                                let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                if !tree_hwnd.is_invalid() {
                                    crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                }
                            }
                        }
                    }
                    else if control_id == 4007 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        if len > 0 {
                            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                            GetWindowTextW(h_edit, &mut buf);
                            if let Ok(w_str) = String::from_utf16(&buf[..len as usize]) {
                                if let Ok(w_val) = w_str.parse::<i32>() {
                                    let safe_w = w_val.max(10);
                                    let btn = &mut state.buttons[btn_idx];
                                    btn.rect.right = btn.rect.left + safe_w;
                                    needs_redraw = true;
                                }
                            }
                        }
                    }
                    else if control_id == 4008 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        if len > 0 {
                            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                            GetWindowTextW(h_edit, &mut buf);
                            if let Ok(h_str) = String::from_utf16(&buf[..len as usize]) {
                                if let Ok(h_val) = h_str.parse::<i32>() {
                                    let safe_h = h_val.max(10);
                                    let btn = &mut state.buttons[btn_idx];
                                    btn.rect.bottom = btn.rect.top + safe_h;
                                    needs_redraw = true;
                                }
                            }
                        }
                    }
                    else if control_id == 4011 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        if len > 0 {
                            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                            GetWindowTextW(h_edit, &mut buf);
                            if let Ok(op_str) = String::from_utf16(&buf[..len as usize]) {
                                if let Ok(op_val) = op_str.parse::<i32>() {
                                    let safe_op = op_val.clamp(0, 255) as u8;
                                    let btn = &mut state.buttons[btn_idx];
                                    if btn.opacity != safe_op {
                                        btn.opacity = safe_op;
                                        needs_redraw = true;
                                        let h_track = GetDlgItem(window, 4003).unwrap_or_default();
                                        if !h_track.is_invalid() {
                                            SendMessageW(h_track, TBM_SETPOS, WPARAM(1), LPARAM(safe_op as isize));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    else if control_id == 4013 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        if len > 0 {
                            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                            GetWindowTextW(h_edit, &mut buf);
                            if let Ok(rad_str) = String::from_utf16(&buf[..len as usize]) {
                                if let Ok(rad_val) = rad_str.parse::<i32>() {
                                    let safe_rad = rad_val.clamp(0, 500);
                                    let btn = &mut state.buttons[btn_idx];
                                    if btn.corner_radius != safe_rad {
                                        btn.corner_radius = safe_rad;
                                        needs_redraw = true;
                                        let h_track = GetDlgItem(window, 4014).unwrap_or_default();
                                        if !h_track.is_invalid() {
                                            SendMessageW(h_track, TBM_SETPOS, WPARAM(1), LPARAM(safe_rad as isize));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    else if control_id == 4015 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        if len > 0 {
                            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                            GetWindowTextW(h_edit, &mut buf);
                            if let Ok(sens_str) = String::from_utf16(&buf[..len as usize]) {
                                if let Ok(sens_val) = sens_str.parse::<f32>() {
                                    let safe_sens = sens_val.clamp(0.1, 10.0);
                                    let btn = &mut state.buttons[btn_idx];
                                    if (btn.sensitivity - safe_sens).abs() > 0.01 {
                                        btn.sensitivity = safe_sens;
                                        needs_redraw = true;
                                        let h_track = GetDlgItem(window, 4016).unwrap_or_default();
                                        if !h_track.is_invalid() {
                                            // 滑块内部使用 100 倍精度表示灵敏度。
                                            SendMessageW(h_track, TBM_SETPOS, WPARAM(1), LPARAM((safe_sens * 100.0) as isize));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if needs_redraw && IsWindow(edit_hwnd).as_bool() {
                        crate::ui::render::force_redraw(edit_hwnd, &mut state);
                    }
                }
            });
            return LRESULT(0);
        }
        WM_HSCROLL => {
            let control_hwnd = HWND(lparam.0 as *mut _);
            let ctrl_id = GetDlgCtrlID(control_hwnd);
            let pos = SendMessageW(control_hwnd, TBM_GETPOS, WPARAM(0), LPARAM(0)).0 as i32;
            let btn_idx = GetWindowLongPtrW(window, GWLP_USERDATA) as usize;
            let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);

            crate::core::wndprocs::APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    if ctrl_id == 4003 {
                        state.buttons[btn_idx].opacity = pos as u8;
                        let h_edit = GetDlgItem(window, 4011).unwrap_or_default();
                        if !h_edit.is_invalid() {
                            let wide_op: Vec<u16> = format!("{}", pos).encode_utf16().chain(std::iter::once(0)).collect();
                            SetWindowTextW(h_edit, PCWSTR(wide_op.as_ptr()));
                        }
                    } else if ctrl_id == 4009 {
                        let safe_w = pos.max(10);
                        state.buttons[btn_idx].rect.right = state.buttons[btn_idx].rect.left + safe_w;
                        let h_edit = GetDlgItem(window, 4007).unwrap_or_default();
                        if !h_edit.is_invalid() {
                            let wide_w: Vec<u16> = format!("{}", safe_w).encode_utf16().chain(std::iter::once(0)).collect();
                            SetWindowTextW(h_edit, PCWSTR(wide_w.as_ptr()));
                        }
                    } else if ctrl_id == 4010 {
                        let safe_h = pos.max(10);
                        state.buttons[btn_idx].rect.bottom = state.buttons[btn_idx].rect.top + safe_h;
                        let h_edit = GetDlgItem(window, 4008).unwrap_or_default();
                        if !h_edit.is_invalid() {
                            let wide_h: Vec<u16> = format!("{}", safe_h).encode_utf16().chain(std::iter::once(0)).collect();
                            SetWindowTextW(h_edit, PCWSTR(wide_h.as_ptr()));
                        }
                    } else if ctrl_id == 4014 {
                        let safe_rad = pos.clamp(0, 500);
                        state.buttons[btn_idx].corner_radius = safe_rad;
                        let h_edit = GetDlgItem(window, 4013).unwrap_or_default();
                        if !h_edit.is_invalid() {
                            let wide_rad: Vec<u16> = format!("{}", safe_rad).encode_utf16().chain(std::iter::once(0)).collect();
                            SetWindowTextW(h_edit, PCWSTR(wide_rad.as_ptr()));
                        }
                    } else if ctrl_id == 4016 {
                        // 将滑块整数值还原为实际灵敏度。
                        let safe_sens = (pos as f32 / 100.0).clamp(0.1, 10.0);
                        state.buttons[btn_idx].sensitivity = safe_sens;
                        let h_edit = GetDlgItem(window, 4015).unwrap_or_default();
                        if !h_edit.is_invalid() {
                            let wide_str: Vec<u16> = format!("{:.1}", safe_sens).encode_utf16().chain(std::iter::once(0)).collect();
                            SetWindowTextW(h_edit, PCWSTR(wide_str.as_ptr()));
                        }
                    }
                    if IsWindow(edit_hwnd).as_bool() {
                        crate::ui::render::force_redraw(edit_hwnd, &mut state);
                    }
                }
            });
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}

// =============================================================================
// 按键组合实例面板 (Combo Instance Window)
// =============================================================================

/// 展示某个组合实例的属性与批量编辑面板。
pub unsafe fn show_combo_instance_window(edit_hwnd: HWND, cat_idx: usize, inst_idx: usize, gid: u32, state: &mut crate::app_state::AppState) {
    let instance = HINSTANCE(GetWindowLongPtrW(edit_hwnd, GWLP_HINSTANCE) as *mut _);
    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
    if sidebar_handle.is_invalid() { return; }
    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
    let right_pane = HWND(GetPropW(sidebar_hwnd, w!("RightPane")).0 as *mut _);
    if right_pane.0.is_null() { return; }

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(0), LPARAM(0));

    let old_handle = GetPropW(edit_hwnd, w!("PropertyHwnd"));
    if !old_handle.is_invalid() { DestroyWindow(HWND(old_handle.0 as *mut _)); }

    let class_name = w!("OmniTouch-ComboInstancePane");
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hInstance: instance, lpszClassName: class_name,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(combo_instance_wndproc),
        hbrBackground: HBRUSH(COLOR_WINDOW.0 as *mut _),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc);

    let mut rect = RECT::default();
    GetClientRect(right_pane, &mut rect);

    let pane_hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(), class_name, None,
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        0, 0, rect.right, rect.bottom, right_pane, None, instance, None,
    ).unwrap();

    SetPropW(pane_hwnd, w!("EditHwnd"), HANDLE(edit_hwnd.0 as _));
    SetPropW(edit_hwnd, w!("PropertyHwnd"), HANDLE(pane_hwnd.0 as _));
    
    SetWindowLongPtrW(pane_hwnd, GWLP_USERDATA, gid as isize);

    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("组合名称:"), WS_CHILD | WS_VISIBLE, 50, 50, 80, 25, pane_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    let edit_combo_name = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), 140, 45, 210, 30, pane_hwnd, HMENU(4024 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_combo_name, Some(edit_enter_subclass_proc), 0, 0);
    
    let custom_name = state.buttons.iter().find(|b| b.group_id == gid).and_then(|b| b.group_name.clone()).unwrap_or_else(|| format!("组合 {}", inst_idx + 1));
    let wide_name: Vec<u16> = custom_name.encode_utf16().chain(std::iter::once(0)).collect();
    SetWindowTextW(edit_combo_name, PCWSTR(wide_name.as_ptr()));

    let combo_names = ["移动(WASD) ", "方向键", "数字键(1-0)", "Xbox手柄", "屏幕键盘", "自定义组合"];
    let title_str = format!("类型: {}", combo_names[cat_idx]);
    let wide_title: Vec<u16> = title_str.encode_utf16().chain(std::iter::once(0)).collect();
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), PCWSTR(wide_title.as_ptr()), WS_CHILD | WS_VISIBLE, 50, 90, 300, 30, pane_hwnd, HMENU(std::ptr::null_mut()), instance, None);

    let count = state.buttons.iter().filter(|b| b.group_id == gid).count();
    let count_str = format!("包含按键: {} 个", count);
    let wide_count: Vec<u16> = count_str.encode_utf16().chain(std::iter::once(0)).collect();
    // 给数量标签分配固定 ID，方便后续实时刷新。
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), PCWSTR(wide_count.as_ptr()), WS_CHILD | WS_VISIBLE, 50, 130, 300, 30, pane_hwnd, HMENU(4026 as *mut _), instance, None);

    let btn_delete = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("删除整组"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 50, 180, 140, 50, pane_hwnd, HMENU(4021 as *mut _), instance, None).unwrap();
    let btn_add = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("添加现有按键"), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 200, 180, 140, 50, pane_hwnd, HMENU(4025 as *mut _), instance, None).unwrap();
    
    let btn_confirm = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("确定"), WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 200, 180, 65, 50, pane_hwnd, HMENU(4027 as *mut _), instance, None).unwrap();
    let btn_cancel = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), w!("取消"), WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 275, 180, 65, 50, pane_hwnd, HMENU(4028 as *mut _), instance, None).unwrap();

    if state.combo_select_mode == Some(gid) {
        let _ = ShowWindow(btn_delete, SW_HIDE);
        let _ = ShowWindow(btn_add, SW_HIDE);
        let _ = ShowWindow(btn_confirm, SW_SHOW);
        let _ = ShowWindow(btn_cancel, SW_SHOW);
    } else {
        let _ = ShowWindow(btn_confirm, SW_HIDE);
        let _ = ShowWindow(btn_cancel, SW_HIDE);
    }

    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("整体缩放:"), WS_CHILD | WS_VISIBLE, 50, 250, 80, 25, pane_hwnd, HMENU(std::ptr::null_mut()), instance, None);
    
    let track_scale = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("msctls_trackbar32"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((TBS_HORZ | TBS_AUTOTICKS) as u32), 130, 245, 200, 30, pane_hwnd, HMENU(4022 as *mut _), instance, None).unwrap();
    SendMessageW(track_scale, TBM_SETRANGE, WPARAM(1), LPARAM(MAKELONG(10, 300) as isize));
    SendMessageW(track_scale, TBM_SETPOS, WPARAM(1), LPARAM(100 as isize));
    
    let edit_scale = CreateWindowExW(WS_EX_CLIENTEDGE, w!("Edit"), None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_NUMBER | ES_AUTOHSCROLL) as u32), 340, 245, 50, 25, pane_hwnd, HMENU(4023 as *mut _), instance, None).unwrap();
    let _ = SetWindowSubclass(edit_scale, Some(edit_enter_subclass_proc), 0, 0);
    SetWindowTextW(edit_scale, w!("100%"));

    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), w!("* 拖动进行缩放，松开滑块后自动复位基准"), WS_CHILD | WS_VISIBLE, 135, 280, 250, 20, pane_hwnd, HMENU(std::ptr::null_mut()), instance, None);

    // 记录上一次缩放比例，便于增量拖动时判断是否需要重算。
    SetPropW(pane_hwnd, w!("LastScale"), HANDLE(100 as isize as *mut _));

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(1), LPARAM(0));
    InvalidateRect(right_pane, None, TRUE);
    let _ = UpdateWindow(right_pane); // 强制父容器彻底擦除旧画面
    
    InvalidateRect(pane_hwnd, None, TRUE);
    let _ = UpdateWindow(pane_hwnd); // 强制面板容器自身重绘

    let _ = EnumChildWindows(pane_hwnd, Some(apply_font_to_child), LPARAM(0));
}

// 记录缩放开始瞬间的原始矩形，避免连续缩放时累计误差。
thread_local! {
    static COMBO_SCALE_BASE: std::cell::RefCell<std::collections::HashMap<usize, (crate::app_state::SerializableRect, i32)>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// 组合实例面板窗口过程，处理名称、缩放和删除等操作。
unsafe extern "system" fn combo_instance_wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // 捕获缩放滑块拖动并按组合中心进行整体缩放。
        WM_HSCROLL => {
            let control_hwnd = HWND(lparam.0 as *mut _);
            let ctrl_id = GetDlgCtrlID(control_hwnd);
            
            if ctrl_id == 4022 {
                let request = (wparam.0 & 0xFFFF) as u32;
                
                // 松开滑块后回到 100%，下一次拖动从当前布局重新开始。
                if request == 8 {
                    SendMessageW(control_hwnd, TBM_SETPOS, WPARAM(1), LPARAM(100));
                    SetPropW(window, w!("LastScale"), HANDLE(100 as isize as *mut _));
                    let h_edit = GetDlgItem(window, 4023).unwrap_or_default();
                    if !h_edit.is_invalid() { let _ = SetWindowTextW(h_edit, w!("100%")); }
                    
                    // 清空快照，下一次拖拽将以此时全新的布局为基准
                    COMBO_SCALE_BASE.with(|c| c.borrow_mut().clear());
                    return LRESULT(0);
                }

                // 拖动过程中提取当前比例
                let pos = SendMessageW(control_hwnd, TBM_GETPOS, WPARAM(0), LPARAM(0)).0 as i32;
                let gid = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);

                let last_scale_handle = GetPropW(window, w!("LastScale"));
                let last_scale = if !last_scale_handle.is_invalid() { last_scale_handle.0 as i32 } else { 100 };
                
                if pos != last_scale && pos > 0 {
                    // 始终相对于初始快照计算绝对倍率，避免逐帧缩放累积误差。
                    let scale_factor = pos as f32 / 100.0;
                    SetPropW(window, w!("LastScale"), HANDLE(pos as isize as *mut _));
                    
                    let h_edit = GetDlgItem(window, 4023).unwrap_or_default();
                    if !h_edit.is_invalid() {
                        let text = format!("{}%", pos);
                        let wide_text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                        let _ = SetWindowTextW(h_edit, PCWSTR(wide_text.as_ptr()));
                    }

                    // 启动底层绝对坐标修改
                    crate::core::wndprocs::APP_STATE.with(|s| {
                        if let Ok(mut state) = s.try_borrow_mut() {
                            COMBO_SCALE_BASE.with(|cache| {
                                let mut base_map = cache.borrow_mut();
                                
                                // 1. 建立快照防线：如果是从 100 开始滑动，说明刚开始拖拽，立刻给全组按键拍个照保存下来！
                                if last_scale == 100 {
                                    base_map.clear();
                                    for (i, btn) in state.buttons.iter().enumerate() {
                                        if btn.group_id == gid {
                                            base_map.insert(i, (btn.rect, btn.corner_radius));
                                        }
                                    }
                                }

                                if base_map.is_empty() { return; }

                                // 2. 找出"快照时"整个按键群的边界，计算出永不偏移的中心原点
                                let mut min_x = i32::MAX; let mut min_y = i32::MAX;
                                let mut max_x = i32::MIN; let mut max_y = i32::MIN;
                                
                                for (_, (rect, _)) in base_map.iter() {
                                    min_x = min_x.min(rect.left); min_y = min_y.min(rect.top);
                                    max_x = max_x.max(rect.right); max_y = max_y.max(rect.bottom);
                                }
                                
                                let cx = (min_x + max_x) as f32 / 2.0;
                                let cy = (min_y + max_y) as f32 / 2.0;
                                
                                // 3. 绝对计算：拿滑块倍率去乘以"快照里的初始值"，覆盖掉当前的按钮
                                for (i, (base_rect, base_rad)) in base_map.iter() {
                                    if let Some(btn) = state.buttons.get_mut(*i) {
                                        let w = (base_rect.right - base_rect.left) as f32 * scale_factor;
                                        let h = (base_rect.bottom - base_rect.top) as f32 * scale_factor;
                                        let center_btn_x = (base_rect.left + base_rect.right) as f32 / 2.0;
                                        let center_btn_y = (base_rect.top + base_rect.bottom) as f32 / 2.0;
                                        
                                        // 绝对放射位移
                                        let new_cx = cx + (center_btn_x - cx) * scale_factor;
                                        let new_cy = cy + (center_btn_y - cy) * scale_factor;
                                        
                                        btn.rect.left = (new_cx - w / 2.0).round() as i32;
                                        btn.rect.right = (new_cx + w / 2.0).round() as i32;
                                        btn.rect.top = (new_cy - h / 2.0).round() as i32;
                                        btn.rect.bottom = (new_cy + h / 2.0).round() as i32;
                                        
                                        // 圆角等比例绝对缩放
                                        btn.corner_radius = (*base_rad as f32 * scale_factor).round() as i32;
                                    }
                                }
                            });
                            
                            // 强制底层重绘，实现丝滑跟随
                            if IsWindow(edit_hwnd).as_bool() {
                                crate::ui::render::force_redraw(edit_hwnd, &mut state);
                            }
                        }
                    });
                }
            }
            return LRESULT(0);
        }

        WM_COMMAND => {
            let control_id = (wparam.0 & 0xFFFF) as i32;
            let notify_code = (wparam.0 >> 16) as u32;

            crate::core::wndprocs::APP_STATE.with(|s| {
                if let Ok(mut state) = s.try_borrow_mut() {
                    let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                    let mut needs_redraw = false;

                    if control_id == 4023 || control_id == 4024 {
                        if notify_code == EN_SETFOCUS {
                            if state.use_system_osk {
                                let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".to_string());
                                let target = format!("{}\\sysnative\\osk.exe", windir);
                                let target = if std::path::Path::new(&target).exists() { target } else { format!("{}\\System32\\osk.exe", windir) };
                                let mut wide_target: Vec<u16> = target.encode_utf16().collect(); wide_target.push(0);
                                let _ = ShellExecuteW(None, w!("open"), PCWSTR(wide_target.as_ptr()), None, None, SW_SHOW);
                            } else {
                                state.osk_visible = true; needs_redraw = true;
                                let _ = SetWindowPos(edit_hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                            }
                        } else if notify_code == EN_KILLFOCUS {
                            if !state.use_system_osk {
                                state.osk_visible = false; needs_redraw = true;
                            }
                            
                            if control_id == 4023 {
                                let h_edit = HWND(lparam.0 as *mut _);
                                let _ = SetWindowTextW(h_edit, w!("100%"));
                            }
                        }
                    }

                    if control_id == 4024 && notify_code == EN_CHANGE as u32 {
                        let h_edit = HWND(lparam.0 as *mut _);
                        let len = GetWindowTextLengthW(h_edit);
                        let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
                        GetWindowTextW(h_edit, &mut buf);
                        if let Ok(new_name) = String::from_utf16(&buf[..len as usize]) {
                            let gid = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                            for btn in &mut state.buttons { if btn.group_id == gid { btn.group_name = Some(new_name.clone()); } }
                            
                            if IsWindow(edit_hwnd).as_bool() {
                                let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                                if !sidebar_handle.is_invalid() {
                                    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                    let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                    if !tree_hwnd.is_invalid() { crate::ui::sync_sidebar_list(tree_hwnd, &mut state); }
                                }
                            }
                        }
                    }
                    if needs_redraw && IsWindow(edit_hwnd).as_bool() { crate::ui::render::force_redraw(edit_hwnd, &mut state); }
                }
            });
            
            if control_id == 4025 && notify_code == BN_CLICKED as u32 {
                let gid = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                crate::core::wndprocs::APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        state.combo_select_mode = Some(gid);
                        state.combo_select_temp = state.buttons.iter().enumerate().filter(|(_, b)| b.group_id == gid).map(|(i, _)| i).collect();
                        
                        let _ = ShowWindow(GetDlgItem(window, 4021).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4025).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4027).unwrap_or_default(), SW_SHOW);
                        let _ = ShowWindow(GetDlgItem(window, 4028).unwrap_or_default(), SW_SHOW);
                        
                        let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                        if IsWindow(edit_hwnd).as_bool() { crate::ui::render::force_redraw(edit_hwnd, &mut state); }
                    }
                });
            }

            if control_id == 4027 && notify_code == BN_CLICKED as u32 {
                let gid = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                crate::core::wndprocs::APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        let mut target_cat = -1;
                        for btn in &state.buttons { if btn.group_id == gid { target_cat = btn.combo_category; break; } }
                        if target_cat == -1 { target_cat = 5; }
                        
                        for btn in &mut state.buttons { if btn.group_id == gid { btn.group_id = 0; btn.combo_category = -1; } }
                        
                        let temp_clone = state.combo_select_temp.clone();
                        for &idx in &temp_clone {
                            if let Some(btn) = state.buttons.get_mut(idx) {
                                btn.group_id = gid; btn.combo_category = target_cat;
                            }
                        }
                        
                        state.combo_select_mode = None; state.combo_select_temp.clear();
                        let _ = ShowWindow(GetDlgItem(window, 4027).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4028).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4021).unwrap_or_default(), SW_SHOW);
                        let _ = ShowWindow(GetDlgItem(window, 4025).unwrap_or_default(), SW_SHOW);
                        
                        let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                        if IsWindow(edit_hwnd).as_bool() {
                            crate::ui::render::force_redraw(edit_hwnd, &mut state);
                            let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                            if !sidebar_handle.is_invalid() {
                                let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                if !tree_hwnd.is_invalid() { crate::ui::sync_sidebar_list(tree_hwnd, &mut state); }
                                
                                let h_count = GetDlgItem(window, 4026).unwrap_or_default();
                                if !h_count.is_invalid() {
                                    let c = state.buttons.iter().filter(|b| b.group_id == gid).count();
                                    let w_c: Vec<u16> = format!("包含按键: {} 个", c).encode_utf16().chain(std::iter::once(0)).collect();
                                    SetWindowTextW(h_count, PCWSTR(w_c.as_ptr()));
                                }
                            }
                        }
                    }
                });
            }

            if control_id == 4028 && notify_code == BN_CLICKED as u32 {
                crate::core::wndprocs::APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        state.combo_select_mode = None; state.combo_select_temp.clear();
                        let _ = ShowWindow(GetDlgItem(window, 4027).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4028).unwrap_or_default(), SW_HIDE);
                        let _ = ShowWindow(GetDlgItem(window, 4021).unwrap_or_default(), SW_SHOW);
                        let _ = ShowWindow(GetDlgItem(window, 4025).unwrap_or_default(), SW_SHOW);
                        
                        let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);
                        if IsWindow(edit_hwnd).as_bool() { crate::ui::render::force_redraw(edit_hwnd, &mut state); }
                    }
                });
            }
            if control_id == 4021 && notify_code == BN_CLICKED as u32 {
                let res = MessageBoxW(window, w!("确定要删除整个按键组合及其包含的所有按键吗？"), w!("二次确认"), MB_YESNO | MB_ICONWARNING);
                
                if res == windows::Win32::UI::WindowsAndMessaging::IDYES {
                    let gid = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                    let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);

                    if gid != 0 {
                        crate::core::wndprocs::APP_STATE.with(|s| {
                            if let Ok(mut state) = s.try_borrow_mut() {
                                state.buttons.retain(|b| b.group_id != gid);
                                
                                if IsWindow(edit_hwnd).as_bool() {
                                    crate::ui::render::force_redraw(edit_hwnd, &mut state);
                                    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                                    if !sidebar_handle.is_invalid() {
                                        let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                        let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                        if !tree_hwnd.is_invalid() {
                                            crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                        }
                                    }
                                }
                            }
                        });
                    }
                    
                    let _ = RemovePropW(edit_hwnd, w!("PropertyHwnd"));
                    let _ = DestroyWindow(window);
                }
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}

// =============================================================================
// 分类新增面板 (Category Add Window)
// =============================================================================

/// 显示当前分类的新增入口面板。
pub unsafe fn show_category_add_window(edit_hwnd: HWND, cat_type: u32, idx: usize, _state: &mut crate::app_state::AppState) {
    let instance = HINSTANCE(GetWindowLongPtrW(edit_hwnd, GWLP_HINSTANCE) as *mut _);
    let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
    if sidebar_handle.is_invalid() { return; }
    let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
    let right_pane = HWND(GetPropW(sidebar_hwnd, w!("RightPane")).0 as *mut _);
    if right_pane.0.is_null() { return; }

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(0), LPARAM(0));

    let old_handle = GetPropW(edit_hwnd, w!("PropertyHwnd"));
    if !old_handle.is_invalid() { DestroyWindow(HWND(old_handle.0 as *mut _)); }

    let class_name = w!("OmniTouch-CategoryPane");
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hInstance: instance, lpszClassName: class_name,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(category_pane_wndproc),
        hbrBackground: HBRUSH(COLOR_WINDOW.0 as *mut _),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc);

    let mut rect = RECT::default();
    GetClientRect(right_pane, &mut rect);

    let pane_hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(), class_name, None,
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        0, 0, rect.right, rect.bottom, right_pane, None, instance, None,
    ).unwrap();

    SetPropW(pane_hwnd, w!("EditHwnd"), HANDLE(edit_hwnd.0 as _));
    SetPropW(edit_hwnd, w!("PropertyHwnd"), HANDLE(pane_hwnd.0 as _));
    
    SetWindowLongPtrW(pane_hwnd, GWLP_USERDATA, ((cat_type << 16) | (idx as u32)) as isize);

    let sidebar_names = ["普通按钮", "单击保持", "滑动触发", "触控板", "绝对鼠标", "自由按键", "鼠标摇杆"];
    let combo_names = ["WASD 移动", "方向键", "数字键 (1-0)", "Xbox 手柄布局", "全键盘开关", "自定义组合"];
    let title = if cat_type == 1 { sidebar_names[idx].to_string() } else { combo_names[idx].to_string() };
    let title_str = format!("当前所选分类: {}", title);
    let wide_title: Vec<u16> = title_str.encode_utf16().chain(std::iter::once(0)).collect();
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Static"), PCWSTR(wide_title.as_ptr()), WS_CHILD | WS_VISIBLE, 50, 50, 300, 30, pane_hwnd, HMENU(std::ptr::null_mut()), instance, None);

    // `cat_type == 1` 表示普通单键分类，其余为组合分类。
    let btn_text = if cat_type == 1 { w!("+ 新增当前分类按键") } else { w!("+ 新增该按键组合") };
    CreateWindowExW(WINDOW_EX_STYLE::default(), w!("Button"), btn_text, WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), 50, 100, 200, 50, pane_hwnd, HMENU(4020 as *mut _), instance, None);

    SendMessageW(right_pane, WM_SETREDRAW, WPARAM(1), LPARAM(0));
    InvalidateRect(right_pane, None, TRUE);
    let _ = UpdateWindow(right_pane); // 强制父容器彻底擦除旧画面
    
    InvalidateRect(pane_hwnd, None, TRUE);
    let _ = UpdateWindow(pane_hwnd); // 强制面板容器自身重绘

    let _ = EnumChildWindows(pane_hwnd, Some(apply_font_to_child), LPARAM(0));
}

/// 分类新增面板窗口过程，负责创建新按键或新组合。
unsafe extern "system" fn category_pane_wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xFFFF) as i32;
            let notify_code = (wparam.0 >> 16) as u32;
            
            if control_id == 4020 && notify_code == BN_CLICKED as u32 {
                let userdata = GetWindowLongPtrW(window, GWLP_USERDATA) as u32;
                let cat_type = userdata >> 16;
                let idx = (userdata & 0xFFFF) as usize;
                let edit_hwnd = HWND(GetPropW(window, w!("EditHwnd")).0 as *mut _);

                crate::core::wndprocs::APP_STATE.with(|s| {
                    if let Ok(mut state) = s.try_borrow_mut() {
                        // 普通单键分类直接创建按键，组合分类则走预设组合逻辑。
                        if cat_type == 1 {
                            let variant = if idx == 6 { crate::app_state::ButtonVariant::Joystick } else { crate::app_state::ButtonVariant::Normal };
                            let (w, h) = if idx == 6 { (150, 150) } else { (100, 100) };
                            let key_code = if idx == 4 { windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON.0 } else { windows::Win32::UI::Input::KeyboardAndMouse::VK_A.0 };
                            let label = if idx == 4 { "绝对鼠标".to_string() } else if idx == 6 { "鼠标摇杆".to_string() } else { "触控板".to_string() };

                            let new_btn = crate::app_state::VirtualButton {
                                rect: crate::app_state::SerializableRect { left: 400, top: 400, right: 400 + w, bottom: 400 + h },
                                key_code, label, opacity: 180, is_pressed: false, group: idx as u8,
                                variant, group_id: 0, joystick_val: (0.0, 0.0), combo_category: -1,
                                group_name: None,
                                corner_radius: if idx == 6 { 75 } else { 25 },
                                sensitivity: 1.0,
                            };
                            state.buttons.push(new_btn);
                        } else {
                            state.add_combo(idx);
                        }
                        
                        if IsWindow(edit_hwnd).as_bool() {
                            crate::ui::render::force_redraw(edit_hwnd, &mut state);
                            let sidebar_handle = GetPropW(edit_hwnd, w!("SidebarHwnd"));
                            if !sidebar_handle.is_invalid() {
                                let sidebar_hwnd = HWND(sidebar_handle.0 as *mut _);
                                let tree_hwnd = GetDlgItem(sidebar_hwnd, 3002).unwrap_or_default();
                                if !tree_hwnd.is_invalid() {
                                    crate::ui::sync_sidebar_list(tree_hwnd, &mut state);
                                }
                            }
                        }
                    }
                });
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}
