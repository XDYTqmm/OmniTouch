//! 文件作用：集中定义程序运行状态、虚拟按键模型、离屏缓冲区以及配置读写逻辑。

use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Write};
use std::ptr;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, WS_EX_LAYERED, SET_WINDOW_POS_FLAGS};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ProgramMode {
    Menu,
    Editing,
    Running,
    Paused,
    ButtonDetail(usize),
    GroupDetail(u32),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct GlobalSettings {
    pub use_system_osk: bool,
    #[serde(default)]
    pub use_virtual_gamepad: bool,
    #[serde(default)]
    pub minimize_to_tray: bool,
}

#[derive(Clone, Copy)]
pub struct TouchRecord {
    pub btn_idx: usize,
    pub last_pos: POINT,
    pub start_pos: POINT,
    pub drag_offset: POINT,
}

/// 根据开关状态注册或移除开机自启项。
pub fn set_auto_start(enable: bool) {
    unsafe {
        let mut hkey = HKEY::default();
        let key_path = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run");
        if RegOpenKeyExW(HKEY_CURRENT_USER, key_path, 0, KEY_ALL_ACCESS, &mut hkey).is_ok() {
            let val_name = w!("OmniTouch");
            if enable {
                if let Ok(exe_path) = std::env::current_exe() {
                    let path_str = exe_path.to_string_lossy().to_string();
                    let mut wide_path: Vec<u16> = path_str.encode_utf16().collect();
                    wide_path.push(0);
                    let byte_len = (wide_path.len() * 2) as u32;
                    let path_bytes = std::slice::from_raw_parts(wide_path.as_ptr() as *const u8, byte_len as usize);
                    let _ = RegSetValueExW(hkey, val_name, 0, REG_SZ, Some(path_bytes));
                }
            } else {
                let _ = RegDeleteValueW(hkey, val_name);
            }
            let _ = RegCloseKey(hkey);
        }
    }
}

/// 读取当前用户是否已启用开机自启。
pub fn get_auto_start() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        let key_path = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run");
        if RegOpenKeyExW(HKEY_CURRENT_USER, key_path, 0, KEY_READ, &mut hkey).is_ok() {
            let val_name = w!("OmniTouch");
            let res = RegQueryValueExW(hkey, val_name, None, None, None, None);
            let _ = RegCloseKey(hkey);
            return res.is_ok();
        }
        false
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct SerializableRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for SerializableRect {
    /// 将 Windows `RECT` 转换为可序列化的矩形结构。
    fn from(r: RECT) -> Self {
        Self { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

impl From<SerializableRect> for RECT {
    /// 将可序列化矩形还原为 Windows `RECT`。
    fn from(s: SerializableRect) -> Self {
        Self { left: s.left, top: s.top, right: s.right, bottom: s.bottom }
    }
}

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize, Debug, Default)]
pub enum ButtonVariant {
    #[default] Normal,
    Joystick,
    Trigger,
    OSKToggle,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VirtualButton {
    pub rect: SerializableRect,
    pub key_code: u16,
    pub label: String,
    #[serde(default = "default_opacity")]
    pub opacity: u8,
    #[serde(skip)]
    pub is_pressed: bool,
    #[serde(default = "default_group")]
    pub group: u8,
    #[serde(default)]
    pub variant: ButtonVariant,
    #[serde(default)]
    pub group_id: u32,
    #[serde(skip)]
    pub joystick_val: (f32, f32),
    #[serde(default = "default_combo_category")]
    pub combo_category: i32,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: i32,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,
}

/// 提供未分组按键的默认组合分类值。
fn default_combo_category() -> i32 { -1 }
/// 提供普通按键的默认圆角半径。
fn default_corner_radius() -> i32 { 25 }
/// 提供普通按键的默认透明度。
fn default_opacity() -> u8 { 180 }
/// 提供普通按键的默认分组。
fn default_group() -> u8 { 0 }
/// 提供触控板类按键的默认灵敏度。
fn default_sensitivity() -> f32 { 1.0 }

impl Default for VirtualButton {
    /// 构造一个用于新建流程的默认虚拟按键。
    fn default() -> Self {
        Self {
            rect: SerializableRect { left: 0, top: 0, right: 100, bottom: 100 },
            key_code: 0, label: "New".into(), opacity: 255, is_pressed: false, group: 0, group_id: 0,
            joystick_val: (0.0, 0.0), variant: ButtonVariant::Normal, combo_category: -1, 
            group_name: None,
            corner_radius: 25,
            sensitivity: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct KeyInfo { pub label: &'static str, pub vk: VIRTUAL_KEY }
pub struct KeyCategory { pub name: &'static str, pub keys: Vec<KeyInfo> }

pub struct OffscreenBuffer {
    pub width: i32, pub height: i32, pub hdc: HDC, pub hbitmap: HBITMAP, pub bits: *mut u32, pub old_bitmap: HGDIOBJ,
    pub d2d_factory: Option<ID2D1Factory>,
    pub dwrite_factory: Option<IDWriteFactory>,
    pub d2d_target: Option<ID2D1DCRenderTarget>,
    pub text_format: Option<IDWriteTextFormat>,
    pub brush_normal: Option<ID2D1SolidColorBrush>,
    pub brush_pressed: Option<ID2D1SolidColorBrush>,
    pub brush_text: Option<ID2D1SolidColorBrush>,
    pub brush_joystick_bg: Option<ID2D1SolidColorBrush>,
    pub brush_joystick_knob: Option<ID2D1SolidColorBrush>,
    pub brush_trigger_fill: Option<ID2D1SolidColorBrush>,
    pub brush_osk_bg: Option<ID2D1SolidColorBrush>,
    pub brush_osk_title: Option<ID2D1SolidColorBrush>,
    pub brush_edit_border: Option<ID2D1SolidColorBrush>,
    pub brush_edit_hover: Option<ID2D1SolidColorBrush>,
    pub brush_edit_normal: Option<ID2D1SolidColorBrush>,
    pub brush_group_border: Option<ID2D1SolidColorBrush>,
    pub brush_group_selected: Option<ID2D1SolidColorBrush>,
}

impl OffscreenBuffer {
    /// 初始化离屏绘制所需的 Direct2D / DirectWrite 资源。
    pub fn new() -> Self {
        let mut d2d_factory = None;
        let mut dwrite_factory = None;
        let mut text_format = None;

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            
            if let Ok(f) = D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
                d2d_factory = Some(f);
            }
            
            if let Ok(f) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) {
                if let Ok(tf) = f.CreateTextFormat(
                    w!("Microsoft YaHei"), None, 
                    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 
                    22.0, w!("zh-cn")
                ) {
                    let _ = tf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                    let _ = tf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                    text_format = Some(tf);
                }
                dwrite_factory = Some(f);
            }
        }

        Self { 
            width: 0, height: 0, hdc: HDC::default(), hbitmap: HBITMAP::default(), bits: ptr::null_mut(), old_bitmap: HGDIOBJ::default(),
            d2d_factory, dwrite_factory, d2d_target: None, text_format,
            brush_normal: None, brush_pressed: None, brush_text: None,
            brush_joystick_bg: None, brush_joystick_knob: None, brush_trigger_fill: None,
            brush_osk_bg: None, brush_osk_title: None,
            brush_edit_border: None, brush_edit_hover: None, brush_edit_normal: None,
            brush_group_border: None, brush_group_selected: None,
        }
    }

    /// 为指定渲染目标创建本模块复用的画刷资源。
    pub unsafe fn init_brushes(&mut self, target: &ID2D1RenderTarget) {
        let color_normal = Common::D2D1_COLOR_F { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
        let color_pressed = Common::D2D1_COLOR_F { r: 0.4, g: 0.4, b: 0.4, a: 1.0 };
        let color_text = Common::D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
        let color_joystick_bg = Common::D2D1_COLOR_F { r: 0.15, g: 0.15, b: 0.15, a: 1.0 };
        let color_joystick_knob = Common::D2D1_COLOR_F { r: 0.7, g: 0.7, b: 0.7, a: 1.0 };
        let color_trigger_fill = Common::D2D1_COLOR_F { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
        let color_osk_bg = Common::D2D1_COLOR_F { r: 0.1, g: 0.1, b: 0.1, a: 0.9 };
        let color_osk_title = Common::D2D1_COLOR_F { r: 0.25, g: 0.25, b: 0.25, a: 1.0 };
        let color_edit_border = Common::D2D1_COLOR_F { r: 1.0, g: 1.0, b: 0.0, a: 1.0 };
        let color_edit_hover = Common::D2D1_COLOR_F { r: 0.0, g: 1.0, b: 1.0, a: 1.0 };
        let color_edit_normal = Common::D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.5 };
        let color_group_border = Common::D2D1_COLOR_F { r: 0.0, g: 1.0, b: 0.0, a: 0.8 };
        let color_group_selected = Common::D2D1_COLOR_F { r: 1.0, g: 1.0, b: 0.0, a: 1.0 };

        self.brush_normal = target.CreateSolidColorBrush(&color_normal as *const _, None).ok();
        self.brush_pressed = target.CreateSolidColorBrush(&color_pressed as *const _, None).ok();
        self.brush_text = target.CreateSolidColorBrush(&color_text as *const _, None).ok();
        self.brush_joystick_bg = target.CreateSolidColorBrush(&color_joystick_bg as *const _, None).ok();
        self.brush_joystick_knob = target.CreateSolidColorBrush(&color_joystick_knob as *const _, None).ok();
        self.brush_trigger_fill = target.CreateSolidColorBrush(&color_trigger_fill as *const _, None).ok();
        self.brush_osk_bg = target.CreateSolidColorBrush(&color_osk_bg as *const _, None).ok();
        self.brush_osk_title = target.CreateSolidColorBrush(&color_osk_title as *const _, None).ok();
        self.brush_edit_border = target.CreateSolidColorBrush(&color_edit_border as *const _, None).ok();
        self.brush_edit_hover = target.CreateSolidColorBrush(&color_edit_hover as *const _, None).ok();
        self.brush_edit_normal = target.CreateSolidColorBrush(&color_edit_normal as *const _, None).ok();
        self.brush_group_border = target.CreateSolidColorBrush(&color_group_border as *const _, None).ok();
        self.brush_group_selected = target.CreateSolidColorBrush(&color_group_selected as *const _, None).ok();
    }

    /// 按目标尺寸重建位图缓冲区并重新绑定 Direct2D 渲染目标。
    pub fn resize(&mut self, w: i32, h: i32) {
        if self.width == w && self.height == h && !self.hdc.is_invalid() { return; }
        unsafe {
            if !self.hdc.is_invalid() { let _ = SelectObject(self.hdc, self.old_bitmap); let _ = DeleteObject(self.hbitmap); let _ = DeleteDC(self.hdc); }
            let screen_dc = GetDC(None); self.hdc = CreateCompatibleDC(screen_dc); let _ = ReleaseDC(None, screen_dc);
            let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
            let mut bits_ptr: *mut c_void = ptr::null_mut();
            self.hbitmap = CreateDIBSection(self.hdc, &bi, DIB_RGB_COLORS, &mut bits_ptr, None, 0).unwrap_or_default();
            self.bits = bits_ptr as *mut u32; self.old_bitmap = SelectObject(self.hdc, self.hbitmap); self.width = w; self.height = h;

            if let Some(factory) = &self.d2d_factory {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0, dpiY: 96.0,
                    ..Default::default()
                };
                if let Ok(target) = factory.CreateDCRenderTarget(&props) {
                    let rect = RECT { left: 0, top: 0, right: w, bottom: h };
                    let _ = target.BindDC(self.hdc, &rect);
                    let target_iface: ID2D1RenderTarget = target.cast().unwrap();
                    self.init_brushes(&target_iface);
                    self.d2d_target = Some(target);
                }
            }
        }
    }

    /// 将离屏缓冲内容提交到分层窗口。
    pub fn present(&self, hwnd: HWND) {
        unsafe {
            let mut blend = BLENDFUNCTION::default(); blend.BlendOp = AC_SRC_OVER as u8; blend.SourceConstantAlpha = 255; blend.AlphaFormat = AC_SRC_ALPHA as u8;
            let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if (ex_style & WS_EX_LAYERED.0 as isize) == 0 {
                ex_style |= WS_EX_LAYERED.0 as isize; SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style);
                let _ = SetWindowPos(hwnd, HWND::default(), 0, 0, 0, 0, SET_WINDOW_POS_FLAGS(0x0001 | 0x0004));
            }
            let _ = UpdateLayeredWindow(hwnd, None, Some(&POINT { x: 0, y: 0 }), Some(&SIZE { cx: self.width, cy: self.height }), self.hdc, Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), UPDATE_LAYERED_WINDOW_FLAGS(0x00000002));
        }
    }
}

pub struct AppState {
    pub mode: ProgramMode,
    pub buttons: Vec<VirtualButton>,
    pub dragging_button_index: Option<usize>,
    pub drag_offset: POINT,
    pub last_mouse_pos: POINT,
    pub key_categories: Vec<KeyCategory>,
    pub font: HFONT,
    pub buffer: OffscreenBuffer,
    pub group_selected: Option<u8>,
    pub touchpad_active_button: Option<usize>,
    pub touchpad_last_pos: POINT,
    pub configs: Vec<(String, RECT)>,
    pub configs_dir: String,
    pub config_selected: Option<usize>,
    pub osk_visible: bool,
    pub osk_rect: RECT,
    pub osk_buttons: Vec<VirtualButton>,
    pub osk_target_text: Option<String>,
    pub osk_drag_offset: POINT,
    pub is_dragging_osk: bool,
    pub group_drag_start_pt: POINT,
    pub active_group_drag_id: Option<u32>,
    pub edit_selected_group_id: Option<u32>,
    pub auto_start: bool,
    pub use_system_osk: bool,
    pub use_virtual_gamepad: bool,
    pub minimize_to_tray: bool,
    pub active_touches: std::collections::HashMap<u32, TouchRecord>,
    pub combo_select_mode: Option<u32>,
    pub combo_select_temp: Vec<usize>,
}

impl AppState {
    /// 创建应用初始状态，并尝试恢复最近一次使用的配置。
    pub fn new() -> Self {
        let mut buttons = Vec::new();
        if let Ok(mut file) = File::open("config.json") {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                if let Ok(saved) = serde_json::from_str::<Vec<VirtualButton>>(&content) { buttons = saved; }
            }
        }
        if buttons.is_empty() {
            buttons.push(VirtualButton { rect: SerializableRect { left: 100, top: 200, right: 200, bottom: 300 }, key_code: VK_A.0, label: "A".to_string(), opacity: 180, is_pressed: false, group: 0, variant: ButtonVariant::Normal, group_id: 0, joystick_val: (0.0, 0.0), combo_category: -1, group_name: None, corner_radius: 25, sensitivity: 1.0 });
        }

        let key_categories = vec![
            KeyCategory { name: "字母 (A-Z)", keys: (0x41..=0x5A).map(|c| KeyInfo { label: Box::leak(format!("{}", (c as u8) as char).into_boxed_str()), vk: VIRTUAL_KEY(c) }).collect() },
            KeyCategory { name: "数字 (0-9)", keys: (0x30..=0x39).map(|c| KeyInfo { label: Box::leak(format!("{}", (c as u8) as char).into_boxed_str()), vk: VIRTUAL_KEY(c) }).collect() },
            KeyCategory { name: "功能键 (F1-F12)", keys: vec![KeyInfo { label: "F1", vk: VK_F1 }, KeyInfo { label: "F2", vk: VK_F2 }, KeyInfo { label: "F3", vk: VK_F3 }, KeyInfo { label: "F4", vk: VK_F4 }, KeyInfo { label: "F5", vk: VK_F5 }, KeyInfo { label: "F6", vk: VK_F6 }, KeyInfo { label: "F7", vk: VK_F7 }, KeyInfo { label: "F8", vk: VK_F8 }, KeyInfo { label: "F9", vk: VK_F9 }, KeyInfo { label: "F10", vk: VK_F10 }, KeyInfo { label: "F11", vk: VK_F11 }, KeyInfo { label: "F12", vk: VK_F12 }] },
            KeyCategory { name: "控制与编辑", keys: vec![KeyInfo { label: "Esc", vk: VK_ESCAPE }, KeyInfo { label: "Tab", vk: VK_TAB }, KeyInfo { label: "Space", vk: VK_SPACE }, KeyInfo { label: "Enter", vk: VK_RETURN }, KeyInfo { label: "Backspace", vk: VK_BACK }, KeyInfo { label: "Up Arrow", vk: VK_UP }, KeyInfo { label: "Down Arrow", vk: VK_DOWN }, KeyInfo { label: "Left Arrow", vk: VK_LEFT }, KeyInfo { label: "Right Arrow", vk: VK_RIGHT }] },
            KeyCategory { name: "游戏手柄", keys: vec![KeyInfo { label: "A", vk: VK_GAMEPAD_A }, KeyInfo { label: "B", vk: VK_GAMEPAD_B }, KeyInfo { label: "X", vk: VK_GAMEPAD_X }, KeyInfo { label: "Y", vk: VK_GAMEPAD_Y }, KeyInfo { label: "RB", vk: VK_GAMEPAD_RIGHT_SHOULDER }, KeyInfo { label: "LB", vk: VK_GAMEPAD_LEFT_SHOULDER }] },
        ];

        let font = unsafe { CreateFontW(22, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, CLEARTYPE_QUALITY.0 as u32, (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, w!("Microsoft YaHei")) };

        let mut s = Self {
            mode: ProgramMode::Paused, buttons, dragging_button_index: None, drag_offset: POINT::default(), last_mouse_pos: POINT::default(),
            key_categories, font, buffer: OffscreenBuffer::new(), group_selected: None, touchpad_active_button: None, touchpad_last_pos: POINT::default(),
            configs: Vec::new(), configs_dir: "configs".to_string(), config_selected: None,
            osk_visible: false, osk_rect: RECT { left: 100, top: 400, right: 900, bottom: 650 }, osk_buttons: Vec::new(), osk_target_text: None, osk_drag_offset: POINT::default(), is_dragging_osk: false,
            group_drag_start_pt: POINT::default(), active_group_drag_id: None, edit_selected_group_id: None, auto_start: false, use_system_osk: false,
            use_virtual_gamepad: false, minimize_to_tray: false,
            active_touches: std::collections::HashMap::new(),
            combo_select_mode: None,
            combo_select_temp: Vec::new(),
        };

        s.ensure_configs_dir();
        s.load_configs();
        if let Ok(entries) = std::fs::read_dir(&s.configs_dir) {
            let (mut latest_name, mut latest_time) = (None, std::time::SystemTime::UNIX_EPOCH);
            for e in entries.flatten() {
                if let Ok(mt) = e.metadata() {
                    if mt.is_file() {
                        if let Some(name) = e.file_name().to_str() {
                            // 跳过设置文件，避免被当作可加载的按键配置。
                            if name.to_lowercase().ends_with(".json") && name.to_lowercase() != "settings.json" {
                                if let Ok(mod_time) = mt.modified() { if mod_time > latest_time { latest_time = mod_time; latest_name = Some(name.to_string()); } }
                            }
                        }
                    }
                }
            }
            if let Some(name) = latest_name {
                if let Some(idx) = s.configs.iter().position(|c| c.0 == name) { s.config_selected = Some(idx); s.load_config_by_index(idx); }
            }
        }
        s.generate_osk_layout();
        s.auto_start = get_auto_start(); s.load_global_settings();
        s
    }

    /// 生成悬浮全键盘的按键布局。
    pub fn generate_osk_layout(&mut self) {
        self.osk_buttons.clear();

        let start_x = 10;
        let start_y = 60;
        let base_w = 50;
        let h = 50;
        let gap = 5;

        let total_width = (base_w * 16) + (gap * 15);

        enum W {
            Auto,
            Fixed(f32),
        }

        let rows = vec![
            vec![
                ("Esc", VK_ESCAPE, W::Fixed(1.25)),
                ("`", VK_OEM_3, W::Fixed(1.0)),
                ("1", VK_1, W::Fixed(1.0)),
                ("2", VK_2, W::Fixed(1.0)),
                ("3", VK_3, W::Fixed(1.0)),
                ("4", VK_4, W::Fixed(1.0)),
                ("5", VK_5, W::Fixed(1.0)),
                ("6", VK_6, W::Fixed(1.0)),
                ("7", VK_7, W::Fixed(1.0)),
                ("8", VK_8, W::Fixed(1.0)),
                ("9", VK_9, W::Fixed(1.0)),
                ("0", VK_0, W::Fixed(1.0)),
                ("-", VK_OEM_MINUS, W::Fixed(1.0)),
                ("=", VK_OEM_PLUS, W::Fixed(1.0)),
                ("⌫", VK_BACK, W::Auto),
            ],
            vec![
                ("Tab", VK_TAB, W::Auto),
                ("Q", VK_Q, W::Fixed(1.0)),
                ("W", VK_W, W::Fixed(1.0)),
                ("E", VK_E, W::Fixed(1.0)),
                ("R", VK_R, W::Fixed(1.0)),
                ("T", VK_T, W::Fixed(1.0)),
                ("Y", VK_Y, W::Fixed(1.0)),
                ("U", VK_U, W::Fixed(1.0)),
                ("I", VK_I, W::Fixed(1.0)),
                ("O", VK_O, W::Fixed(1.0)),
                ("P", VK_P, W::Fixed(1.0)),
                ("[", VK_OEM_4, W::Fixed(1.0)),
                ("]", VK_OEM_6, W::Fixed(1.0)),
                ("\\", VK_OEM_5, W::Fixed(1.0)),
                ("DEL", VK_DELETE, W::Fixed(1.25)),
            ],
            vec![
                ("Caps", VK_CAPITAL, W::Fixed(2.25)),
                ("A", VK_A, W::Fixed(1.0)),
                ("S", VK_S, W::Fixed(1.0)),
                ("D", VK_D, W::Fixed(1.0)),
                ("F", VK_F, W::Fixed(1.0)),
                ("G", VK_G, W::Fixed(1.0)),
                ("H", VK_H, W::Fixed(1.0)),
                ("J", VK_J, W::Fixed(1.0)),
                ("K", VK_K, W::Fixed(1.0)),
                ("L", VK_L, W::Fixed(1.0)),
                (";", VK_OEM_1, W::Fixed(1.0)),
                ("'", VK_OEM_7, W::Fixed(1.0)),
                ("Enter", VK_RETURN, W::Auto),
            ],
            vec![
                ("Shift", VK_LSHIFT, W::Auto),
                ("Z", VK_Z, W::Fixed(1.0)),
                ("X", VK_X, W::Fixed(1.0)),
                ("C", VK_C, W::Fixed(1.0)),
                ("V", VK_V, W::Fixed(1.0)),
                ("B", VK_B, W::Fixed(1.0)),
                ("N", VK_N, W::Fixed(1.0)),
                ("M", VK_M, W::Fixed(1.0)),
                (",", VK_OEM_COMMA, W::Fixed(1.0)),
                (".", VK_OEM_PERIOD, W::Fixed(1.0)),
                ("/", VK_OEM_2, W::Fixed(1.0)),
                ("↑", VK_UP, W::Fixed(1.0)),
                ("Shift", VK_RSHIFT, W::Auto),
            ],
            vec![
                ("Ctrl", VK_LCONTROL, W::Fixed(1.25)),
                ("Win", VK_LWIN, W::Fixed(1.25)),
                ("Alt", VK_LMENU, W::Fixed(1.25)),
                ("Space", VK_SPACE, W::Auto),
                ("Alt", VK_RMENU, W::Fixed(1.25)),
                ("Ctrl", VK_RCONTROL, W::Fixed(1.25)),
                ("←", VK_LEFT, W::Fixed(1.0)),
                ("↓", VK_DOWN, W::Fixed(1.0)),
                ("→", VK_RIGHT, W::Fixed(1.0)),
                ("Menu", VK_APPS, W::Fixed(1.25)),
            ],
        ];

        let mut current_y = start_y;

        for row in rows {
            let mut current_x = start_x;

            let mut fixed_width_total = 0;
            let mut auto_key_count = 0;
            let gap_total = (row.len() - 1) as i32 * gap;

            for (_, _, w_type) in &row {
                match w_type {
                    W::Fixed(m) => fixed_width_total += (base_w as f32 * m) as i32,
                    W::Auto => auto_key_count += 1,
                }
            }

            let remaining_width = total_width - fixed_width_total - gap_total;
            let auto_width = if auto_key_count > 0 {
                remaining_width / auto_key_count
            } else {
                0
            };

            for (_i, (label, vk, w_type)) in row.into_iter().enumerate() {
                let w = match w_type {
                    W::Fixed(m) => (base_w as f32 * m) as i32,
                    W::Auto => auto_width,
                };

                self.osk_buttons.push(VirtualButton {
                    rect: SerializableRect {
                        left: current_x,
                        top: current_y,
                        right: current_x + w,
                        bottom: current_y + h,
                    },
                    key_code: vk.0,
                    label: label.to_string(),
                    opacity: 200,
                    ..Default::default()
                });

                current_x += w + gap;
            }
            current_y += h + gap;
        }

        self.osk_rect.right = self.osk_rect.left + total_width + 20;
        self.osk_rect.bottom = self.osk_rect.top + current_y + 10;
    }

    /// 按预设类型批量添加一个按键组合。
    ///
    /// `combo_idx` 用于区分 WASD、方向键、手柄布局等预设模板。
    pub fn add_combo(&mut self, combo_idx: usize) {
        let center_x = 500;
        let center_y = 400;
        let mut new_btns = Vec::new();
        let gid = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u32;

        match combo_idx {
            0 => {
                let keys = [
                    ('W', VK_W, 100, 0),
                    ('A', VK_A, 0, 100),
                    ('S', VK_S, 100, 100),
                    ('D', VK_D, 200, 100),
                ];
                for (lbl, vk, dx, dy) in keys {
                    self.buttons.push(self.create_btn(
                        center_x + dx,
                        center_y + dy,
                        100,
                        100,
                        lbl.to_string(),
                        vk.0,
                        gid,
                        ButtonVariant::Normal,
                        combo_idx as i32,
                        2,
                    ));
                }
            }
            1 => {
                let arrows = [
                    ("↑", VK_UP, 100, 0),
                    ("←", VK_LEFT, 0, 100),
                    ("↓", VK_DOWN, 100, 100),
                    ("→", VK_RIGHT, 200, 100),
                ];
                for (lbl, vk, dx, dy) in arrows {
                    new_btns.push(self.create_btn(
                        center_x + dx,
                        center_y + dy,
                        100,
                        100,
                        lbl.to_string(),
                        vk.0,
                        gid,
                        ButtonVariant::Normal,
                        combo_idx as i32,
                        2,
                    ));
                }
            }
            2 => {
                let w = 50;
                let h = 50;
                for i in 0..10 {
                    let k = if i == 9 { 0x30 } else { 0x31 + i as u16 };
                    let l = if i == 9 {
                        "0".to_string()
                    } else {
                        format!("{}", i + 1)
                    };
                    new_btns.push(self.create_btn(
                        center_x - 250 + i * 55,
                        center_y,
                        w,
                        h,
                        l,
                        k,
                        gid,
                        ButtonVariant::Normal,
                        combo_idx as i32,
                        0,
                    ));
                }
            }
            3 => {
                self.buttons.push(self.create_btn(
                    center_x - 200,
                    center_y + 140,
                    80,
                    80,
                    "L-Stick".into(),
                    0,
                    gid,
                    ButtonVariant::Joystick,
                    combo_idx as i32,
                    0,
                ));
                self.buttons.push(self.create_btn(
                    center_x + 280,
                    center_y + 140,
                    80,
                    80,
                    "R-Stick".into(),
                    0,
                    gid,
                    ButtonVariant::Joystick,
                    combo_idx as i32,
                    0,
                ));
                self.buttons.push(self.create_btn(
                    center_x - 180,
                    center_y - 140,
                    60,
                    80,
                    "LT".into(),
                    VK_GAMEPAD_LEFT_TRIGGER.0,
                    gid,
                    ButtonVariant::Trigger,
                    combo_idx as i32,
                    0,
                ));
                self.buttons.push(self.create_btn(
                    center_x + 280,
                    center_y - 140,
                    60,
                    80,
                    "RT".into(),
                    VK_GAMEPAD_RIGHT_TRIGGER.0,
                    gid,
                    ButtonVariant::Trigger,
                    combo_idx as i32,
                    0,
                ));
                let abxy = [
                    ("A", VK_GAMEPAD_A, 160, 120),
                    ("B", VK_GAMEPAD_B, 210, 70),
                    ("X", VK_GAMEPAD_X, 110, 70),
                    ("Y", VK_GAMEPAD_Y, 160, 20),
                ];
                for (lbl, vk, dx, dy) in abxy {
                    self.buttons.push(self.create_btn(
                        center_x + 60 + dx,
                        center_y - 20 + dy,
                        50,
                        50,
                        lbl.into(),
                        vk.0,
                        gid,
                        ButtonVariant::Normal,
                        combo_idx as i32,
                        2,
                    ));
                }
                self.buttons.push(self.create_btn(
                    center_x - 180,
                    center_y - 60,
                    60,
                    50,
                    "LB".into(),
                    VK_GAMEPAD_LEFT_SHOULDER.0,
                    gid,
                    ButtonVariant::Normal,
                    combo_idx as i32,
                    2,
                ));
                self.buttons.push(self.create_btn(
                    center_x + 280,
                    center_y - 60,
                    60,
                    50,
                    "RB".into(),
                    VK_GAMEPAD_RIGHT_SHOULDER.0,
                    gid,
                    ButtonVariant::Normal,
                    combo_idx as i32,
                    2,
                ));
                let dpad = [
                    ("Up", VK_GAMEPAD_DPAD_UP, -120, 0),
                    ("Down", VK_GAMEPAD_DPAD_DOWN, -120, 100),
                    ("Left", VK_GAMEPAD_DPAD_LEFT, -170, 50),
                    ("Right", VK_GAMEPAD_DPAD_RIGHT, -70, 50),
                ];
                for (lbl, vk, dx, dy) in dpad {
                    self.buttons.push(self.create_btn(
                        center_x + dx,
                        center_y + dy,
                        50,
                        50,
                        lbl.into(),
                        vk.0,
                        gid,
                        ButtonVariant::Normal,
                        combo_idx as i32,
                        2,
                    ));
                }
                self.buttons.push(self.create_btn(
                    center_x + 20,
                    center_y - 120,
                    60,
                    50,
                    "Back".into(),
                    VK_GAMEPAD_VIEW.0,
                    gid,
                    ButtonVariant::Normal,
                    combo_idx as i32,
                    2,
                ));
                self.buttons.push(self.create_btn(
                    center_x + 80,
                    center_y - 120,
                    60,
                    50,
                    "Start".into(),
                    VK_GAMEPAD_MENU.0,
                    gid,
                    ButtonVariant::Normal,
                    combo_idx as i32,
                    2,
                ));
            }
            4 => {
                self.buttons.push(self.create_btn(
                    center_x,
                    center_y,
                    50,
                    50,
                    "键盘".into(),
                    0,
                    gid,
                    ButtonVariant::OSKToggle,
                    combo_idx as i32,
                    5,
                ));
            }
            // 自定义组合默认放入一个普通占位按键，便于后续编辑。
            5 => {
                self.buttons.push(self.create_btn(
                    center_x,
                    center_y,
                    100,
                    100,
                    "New".into(),
                    VK_A.0,
                    gid,
                    ButtonVariant::Normal,
                    combo_idx as i32,
                    0,
                ));
            }
            _ => {}
        }
        self.buttons.extend(new_btns);
    }

    /// 创建一个带默认样式的虚拟按键。
    ///
    /// `x/y/w/h` 表示按键矩形的位置和尺寸，`gid` 表示组合实例 ID，
    /// `combo_category` 表示预设分类，`group` 表示交互分组。
    pub fn create_btn(&self, x: i32, y: i32, w: i32, h: i32, label: String, code: u16, gid: u32, variant: ButtonVariant, combo_category: i32, group: u8) -> VirtualButton { VirtualButton { rect: SerializableRect { left: x, top: y, right: x + w, bottom: y + h }, key_code: code, label, opacity: 180, is_pressed: false, group, variant, group_id: gid, joystick_val: (0.0, 0.0), combo_category, group_name: None, corner_radius: 25, sensitivity: 1.0 } }
    /// 将当前按键布局保存到默认配置文件。
    pub fn save_config(&mut self) { for btn in self.buttons.iter_mut() { if btn.label.trim().is_empty() { btn.label = "未命名".to_string(); } } if let Ok(content) = serde_json::to_string_pretty(&self.buttons) { 
        // 先写入临时文件，再原子替换正式文件，避免中断时损坏配置。
        let temp_path = "config.json.tmp";
        if let Ok(mut f) = File::create(temp_path) {
            if f.write_all(content.as_bytes()).is_ok() {
                let _ = std::fs::rename(temp_path, "config.json");
            }
        }
    } }
    /// 确保配置目录存在。
    pub fn ensure_configs_dir(&self) { let _ = std::fs::create_dir_all(&self.configs_dir); }
    /// 重新扫描配置目录并刷新内存中的配置列表。
    pub fn load_configs(&mut self) { 
        self.ensure_configs_dir(); 
        self.configs.clear(); 
        if let Ok(entries) = std::fs::read_dir(&self.configs_dir) { 
            for e in entries.flatten() { 
                if let Ok(mt) = e.metadata() { 
                    if mt.is_file() { 
                        if let Some(name) = e.file_name().to_str().map(|s| s.to_string()) { 
                            // 跳过设置文件，避免显示在用户配置列表中。
                            if name.to_lowercase().ends_with(".json") && name.to_lowercase() != "settings.json" { 
                                self.configs.push((name, RECT::default())); 
                            } 
                        } 
                    } 
                } 
            } 
        } 
    }
    /// 按索引加载指定配置文件中的按键数据。
    pub fn load_config_by_index(&mut self, idx: usize) { if idx >= self.configs.len() { return; } let path = format!("{}/{}", self.configs_dir, self.configs[idx].0); if let Ok(mut f) = File::open(&path) { let mut s = String::new(); if f.read_to_string(&mut s).is_ok() { if let Ok(saved) = serde_json::from_str::<Vec<VirtualButton>>(&s) { self.buttons = saved; } } } }
    /// 复制指定配置，并自动生成不冲突的新文件名。
    pub fn copy_config(&mut self, idx: usize) { if idx >= self.configs.len() { return; } let name = &self.configs[idx].0; let src = format!("{}/{}", self.configs_dir, name); let base = name.trim_end_matches(".json"); let mut dst = format!("{}/{}_copy.json", self.configs_dir, base); let mut i = 1; while std::path::Path::new(&dst).exists() { dst = format!("{}/{}_copy{}.json", self.configs_dir, base, i); i += 1; } let _ = std::fs::copy(&src, &dst); self.load_configs(); }
    /// 删除指定配置，并清空当前选中状态。
    pub fn delete_config(&mut self, idx: usize) { if idx >= self.configs.len() { return; } let path = format!("{}/{}", self.configs_dir, self.configs[idx].0); let _ = std::fs::remove_file(&path); self.load_configs(); self.config_selected = None; }
    /// 创建一个空白配置文件并返回生成后的文件名。
    pub fn new_config(&mut self) -> Option<String> { 
        self.ensure_configs_dir(); 
        
        let mut base_name = String::from("新配置");
        let mut filename = format!("{}.json", base_name);
        let mut i = 1; 
        
        // 递增检查，直到找到一个不存在的文件名
        while std::path::Path::new(&format!("{}/{}", self.configs_dir, filename)).exists() { 
            base_name = format!("新配置{}", i);
            filename = format!("{}.json", base_name); 
            i += 1; 
        } 
        
        // 清空内存中的残留按键，并写入空数组作为新配置的初始内容。
        self.buttons.clear();
        let default_content = "[]";
        
        if let Ok(mut f) = File::create(format!("{}/{}", self.configs_dir, filename)) { 
            let _ = f.write_all(default_content.as_bytes()); 
            self.load_configs(); 
            return Some(filename); 
        } 
        None 
    }
    /// 打开当前配置目录。
    pub fn open_configs_folder(&self) { let _ = std::process::Command::new("explorer").arg(&self.configs_dir).spawn(); }
    /// 优先保存到当前选中配置；未选中时退回默认配置文件。
    pub fn save_to_selected(&mut self) { if let Some(idx) = self.config_selected { if idx < self.configs.len() { self.ensure_configs_dir(); let name = &self.configs[idx].0; let path = format!("{}/{}", self.configs_dir, name); if let Ok(content) = serde_json::to_string_pretty(&self.buttons) { 
        // 先写入临时文件，再替换目标文件，减少保存中断带来的风险。
        let temp_path = format!("{}.tmp", path);
        if let Ok(mut f) = File::create(&temp_path) {
            if f.write_all(content.as_bytes()).is_ok() {
                let _ = std::fs::rename(&temp_path, &path);
            }
        }
    } return; } } self.save_config(); }
    /// 从设置文件加载全局开关状态。
    pub fn load_global_settings(&mut self) { self.ensure_configs_dir(); let path = format!("{}/settings.json", self.configs_dir); if let Ok(mut f) = File::open(&path) { let mut s = String::new(); if f.read_to_string(&mut s).is_ok() { if let Ok(gs) = serde_json::from_str::<GlobalSettings>(&s) { self.use_system_osk = gs.use_system_osk; self.use_virtual_gamepad = gs.use_virtual_gamepad; self.minimize_to_tray = gs.minimize_to_tray; } } } }
    /// 将全局开关状态写回设置文件。
    pub fn save_global_settings(&self) { let path = format!("{}/settings.json", self.configs_dir); if let Ok(gs) = serde_json::to_string_pretty(&GlobalSettings { use_system_osk: self.use_system_osk, use_virtual_gamepad: self.use_virtual_gamepad, minimize_to_tray: self.minimize_to_tray }) { 
        // 设置文件同样采用临时文件替换，避免写入中断。
        let temp_path = format!("{}.tmp", path);
        if let Ok(mut f) = File::create(&temp_path) {
            if f.write_all(gs.as_bytes()).is_ok() {
                let _ = std::fs::rename(&temp_path, &path);
            }
        }
    } }

    /// 释放所有按下中的虚拟按键，并重置触控相关状态。
    pub fn emergency_release_all_keys(&mut self) {
        unsafe {
            for btn in &mut self.buttons {
                if btn.is_pressed {
                    btn.is_pressed = false;
                    crate::input::handler::simulate_key(windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(btn.key_code), false);
                }
                btn.joystick_val = (0.0, 0.0);
            }
            for osk_btn in &mut self.osk_buttons {
                if osk_btn.is_pressed {
                    osk_btn.is_pressed = false;
                    crate::input::handler::simulate_key(windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(osk_btn.key_code), false);
                }
            }
            if self.use_virtual_gamepad {
                crate::input::vigem_wrapper::sync_gamepad(&self.buttons);
            }
            crate::core::event_handler::close_all_osk(self);
            self.active_touches.clear();
            self.dragging_button_index = None;
            self.touchpad_active_button = None;
        }
    }

    pub fn has_unsaved_changes(&self) -> bool {
        if let Some(idx) = self.config_selected {
            if idx < self.configs.len() {
                let path = format!("{}/{}", self.configs_dir, self.configs[idx].0);

                let file_content = std::fs::read_to_string(&path).unwrap_or_default();

                let saved_buttons: Vec<VirtualButton> = serde_json::from_str(&file_content).unwrap_or_default();

                let current_json = serde_json::to_string(&self.buttons).unwrap_or_default();
                let saved_json = serde_json::to_string(&saved_buttons).unwrap_or_default();

                return current_json != saved_json;
            }
        }
        false
    }
}
