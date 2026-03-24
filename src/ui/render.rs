//! 文件作用：负责悬浮层中虚拟按键、编辑态边框和 OSK 的 Direct2D 绘制。

use crate::app_state::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Interface;

/// 按当前程序状态重绘整张悬浮层画面。
pub unsafe fn force_redraw(hwnd: HWND, state: &mut AppState) {
    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);

    state.buffer.resize(screen_w, screen_h);

    let dc_target = match &state.buffer.d2d_target {
        Some(t) => t,
        None => {
            state.buffer.present(hwnd);
            return;
        }
    };

    let target: ID2D1RenderTarget = dc_target.cast().unwrap();
    
    target.BeginDraw();
    // 全屏背景保持完全透明，便于将点击透传给底层窗口。
    target.Clear(Some(&Common::D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

    let brush_normal = match &state.buffer.brush_normal {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_pressed = match &state.buffer.brush_pressed {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_text = match &state.buffer.brush_text {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_joystick_bg = match &state.buffer.brush_joystick_bg {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_joystick_knob = match &state.buffer.brush_joystick_knob {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_trigger_fill = match &state.buffer.brush_trigger_fill {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_osk_bg = match &state.buffer.brush_osk_bg {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_osk_title = match &state.buffer.brush_osk_title {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_edit_border = match &state.buffer.brush_edit_border {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_edit_hover = match &state.buffer.brush_edit_hover {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_edit_normal = match &state.buffer.brush_edit_normal {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_group_border = match &state.buffer.brush_group_border {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };
    let brush_group_selected = match &state.buffer.brush_group_selected {
        Some(b) => b,
        None => { let _ = target.EndDraw(None, None); state.buffer.present(hwnd); return; }
    };

    match state.mode {
        ProgramMode::Menu | ProgramMode::Running | ProgramMode::Paused => {
            if state.mode != ProgramMode::Paused {
                draw_running_d2d(&target, state, brush_normal, brush_pressed, brush_text, brush_joystick_bg, brush_joystick_knob, brush_trigger_fill);
            }
            if state.osk_visible {
                draw_osk_d2d(&target, state, brush_osk_bg, brush_osk_title, brush_normal, brush_pressed, brush_text);
            }
        }
        ProgramMode::Editing => {
            draw_editing_d2d(&target, state, brush_normal, brush_pressed, brush_text, brush_joystick_bg, brush_joystick_knob, brush_trigger_fill, brush_edit_border, brush_edit_hover, brush_edit_normal, brush_group_border, brush_group_selected);
            if state.osk_visible {
                draw_osk_d2d(&target, state, brush_osk_bg, brush_osk_title, brush_normal, brush_pressed, brush_text);
            }
        }
        ProgramMode::ButtonDetail(_) | ProgramMode::GroupDetail(_) => {
            draw_editing_d2d(&target, state, brush_normal, brush_pressed, brush_text, brush_joystick_bg, brush_joystick_knob, brush_trigger_fill, brush_edit_border, brush_edit_hover, brush_edit_normal, brush_group_border, brush_group_selected);
            if state.osk_visible {
                draw_osk_d2d(&target, state, brush_osk_bg, brush_osk_title, brush_normal, brush_pressed, brush_text);
            }
        }
    }

    let _ = target.EndDraw(None, None);
    state.buffer.present(hwnd);
}

/// 在指定矩形中绘制文本。
unsafe fn draw_text_d2d(
    target: &ID2D1RenderTarget,
    text: &str,
    rect: &Common::D2D_RECT_F,
    text_format: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
) {
    if text.is_empty() {
        return;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return;
    }
    target.DrawText(
        &wide,
        text_format,
        rect as *const _,
        brush,
        D2D1_DRAW_TEXT_OPTIONS_NONE,
        DWRITE_MEASURING_MODE_NATURAL,
    );
}

/// 绘制普通矩形按键及其编辑态边框。
unsafe fn draw_button_d2d(
    target: &ID2D1RenderTarget,
    btn: &VirtualButton,
    text_format: Option<&IDWriteTextFormat>,
    brush_normal: &ID2D1SolidColorBrush,
    brush_pressed: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    is_editing: bool,
    is_selected: bool,
    is_hover: bool,
    edit_border: &ID2D1SolidColorBrush,
    edit_hover: &ID2D1SolidColorBrush,
    edit_normal: &ID2D1SolidColorBrush,
) {
    let alpha = if is_editing { btn.opacity.max(80) as f32 / 255.0 } else { btn.opacity as f32 / 255.0 };
    
    let rect = Common::D2D_RECT_F {
        left: btn.rect.left as f32,
        top: btn.rect.top as f32,
        right: btn.rect.right as f32,
        bottom: btn.rect.bottom as f32,
    };

    let rounded_rect = D2D1_ROUNDED_RECT {
        rect,
        radiusX: btn.corner_radius as f32,
        radiusY: btn.corner_radius as f32,
    };

    let current_brush = if btn.is_pressed { brush_pressed } else { brush_normal };
    current_brush.SetOpacity(alpha);
    target.FillRoundedRectangle(&rounded_rect, current_brush);

    if is_editing {
        let border_brush = if is_selected {
            edit_border
        } else if is_hover {
            edit_hover
        } else {
            edit_normal
        };
        border_brush.SetOpacity(if is_selected || is_hover { 1.0 } else { 0.5 });
        
        let border_rect = Common::D2D_RECT_F {
            left: rect.left - 2.0,
            top: rect.top - 2.0,
            right: rect.right + 2.0,
            bottom: rect.bottom + 2.0,
        };
        let border_rounded = D2D1_ROUNDED_RECT {
            rect: border_rect,
            radiusX: btn.corner_radius as f32 + 2.0,
            radiusY: btn.corner_radius as f32 + 2.0,
        };
        target.DrawRoundedRectangle(&border_rounded, border_brush, if is_selected { 3.0 } else { 1.0 }, None);
    }

    if let Some(tf) = text_format {
        let label = if btn.label.is_empty() { "?" } else { &btn.label };
        brush_text.SetOpacity(alpha);
        draw_text_d2d(target, label, &rect, tf, brush_text);
    }
}

/// 绘制摇杆按键及其摇杆帽位置。
unsafe fn draw_joystick_d2d(
    target: &ID2D1RenderTarget,
    btn: &VirtualButton,
    text_format: Option<&IDWriteTextFormat>,
    brush_bg: &ID2D1SolidColorBrush,
    brush_knob: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    is_editing: bool,
    is_selected: bool,
    is_hover: bool,
    edit_border: &ID2D1SolidColorBrush,
    edit_hover: &ID2D1SolidColorBrush,
    edit_normal: &ID2D1SolidColorBrush,
) {
    let alpha = if is_editing { btn.opacity.max(80) as f32 / 255.0 } else { btn.opacity as f32 / 255.0 };
    
    let rect = Common::D2D_RECT_F {
        left: btn.rect.left as f32,
        top: btn.rect.top as f32,
        right: btn.rect.right as f32,
        bottom: btn.rect.bottom as f32,
    };

    let rounded_rect = D2D1_ROUNDED_RECT {
        rect,
        radiusX: btn.corner_radius as f32,
        radiusY: btn.corner_radius as f32,
    };

    brush_bg.SetOpacity(alpha * 0.5);
    target.FillRoundedRectangle(&rounded_rect, brush_bg);

    let cx = (rect.left + rect.right) / 2.0;
    let cy = (rect.top + rect.bottom) / 2.0;
    let r_x = (rect.right - rect.left) / 2.0;
    let r_y = (rect.bottom - rect.top) / 2.0;
    let knob_x = cx + btn.joystick_val.0 * r_x;
    let knob_y = cy + btn.joystick_val.1 * r_y;

    let knob_size = 20.0;
    let knob_rounded = D2D1_ROUNDED_RECT {
        rect: Common::D2D_RECT_F {
            left: knob_x - knob_size,
            top: knob_y - knob_size,
            right: knob_x + knob_size,
            bottom: knob_y + knob_size,
        },
        radiusX: knob_size,
        radiusY: knob_size,
    };
    brush_knob.SetOpacity(alpha * 0.9);
    target.FillRoundedRectangle(&knob_rounded, brush_knob);

    if is_editing {
        let border_brush = if is_selected {
            edit_border
        } else if is_hover {
            edit_hover
        } else {
            edit_normal
        };
        border_brush.SetOpacity(if is_selected || is_hover { 1.0 } else { 0.5 });
        
        let border_rect = Common::D2D_RECT_F {
            left: rect.left - 2.0,
            top: rect.top - 2.0,
            right: rect.right + 2.0,
            bottom: rect.bottom + 2.0,
        };
        let border_rounded = D2D1_ROUNDED_RECT {
            rect: border_rect,
            radiusX: btn.corner_radius as f32 + 2.0,
            radiusY: btn.corner_radius as f32 + 2.0,
        };
        target.DrawRoundedRectangle(&border_rounded, border_brush, if is_selected { 3.0 } else { 1.0 }, None);
    }

    if let Some(tf) = text_format {
        let label = if btn.label.is_empty() { "?" } else { &btn.label };
        brush_text.SetOpacity(alpha * 0.5);
        draw_text_d2d(target, label, &rect, tf, brush_text);
    }
}

/// 绘制扳机按键及其模拟值填充条。
unsafe fn draw_trigger_d2d(
    target: &ID2D1RenderTarget,
    btn: &VirtualButton,
    text_format: Option<&IDWriteTextFormat>,
    brush_bg: &ID2D1SolidColorBrush,
    brush_fill: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    is_editing: bool,
    is_selected: bool,
    is_hover: bool,
    edit_border: &ID2D1SolidColorBrush,
    edit_hover: &ID2D1SolidColorBrush,
    edit_normal: &ID2D1SolidColorBrush,
) {
    let alpha = if is_editing { btn.opacity.max(80) as f32 / 255.0 } else { btn.opacity as f32 / 255.0 };
    
    let rect = Common::D2D_RECT_F {
        left: btn.rect.left as f32,
        top: btn.rect.top as f32,
        right: btn.rect.right as f32,
        bottom: btn.rect.bottom as f32,
    };

    let rounded_rect = D2D1_ROUNDED_RECT {
        rect,
        radiusX: btn.corner_radius as f32,
        radiusY: btn.corner_radius as f32,
    };

    brush_bg.SetOpacity(alpha);
    target.FillRoundedRectangle(&rounded_rect, brush_bg);

    let h = rect.bottom - rect.top;
    let fill_h = h * btn.joystick_val.1;
    let fill_rect = Common::D2D_RECT_F {
        left: rect.left,
        top: rect.bottom - fill_h,
        right: rect.right,
        bottom: rect.bottom,
    };
    let fill_rounded = D2D1_ROUNDED_RECT {
        rect: fill_rect,
        radiusX: btn.corner_radius as f32,
        radiusY: btn.corner_radius as f32,
    };
    brush_fill.SetOpacity(alpha * 0.8);
    target.FillRoundedRectangle(&fill_rounded, brush_fill);

    if is_editing {
        let border_brush = if is_selected {
            edit_border
        } else if is_hover {
            edit_hover
        } else {
            edit_normal
        };
        border_brush.SetOpacity(if is_selected || is_hover { 1.0 } else { 0.5 });
        
        let border_rect = Common::D2D_RECT_F {
            left: rect.left - 2.0,
            top: rect.top - 2.0,
            right: rect.right + 2.0,
            bottom: rect.bottom + 2.0,
        };
        let border_rounded = D2D1_ROUNDED_RECT {
            rect: border_rect,
            radiusX: btn.corner_radius as f32 + 2.0,
            radiusY: btn.corner_radius as f32 + 2.0,
        };
        target.DrawRoundedRectangle(&border_rounded, border_brush, if is_selected { 3.0 } else { 1.0 }, None);
    }

    if let Some(tf) = text_format {
        let label = if btn.label.is_empty() { "?" } else { &btn.label };
        brush_text.SetOpacity(alpha);
        draw_text_d2d(target, label, &rect, tf, brush_text);
    }
}

/// 按按键类型分派到具体绘制函数。
unsafe fn draw_vbutton_d2d(
    target: &ID2D1RenderTarget,
    btn: &VirtualButton,
    text_format: Option<&IDWriteTextFormat>,
    brush_normal: &ID2D1SolidColorBrush,
    brush_pressed: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    brush_joystick_bg: &ID2D1SolidColorBrush,
    brush_joystick_knob: &ID2D1SolidColorBrush,
    brush_trigger_fill: &ID2D1SolidColorBrush,
    is_editing: bool,
    is_selected: bool,
    is_hover: bool,
    edit_border: &ID2D1SolidColorBrush,
    edit_hover: &ID2D1SolidColorBrush,
    edit_normal: &ID2D1SolidColorBrush,
) {
    match btn.variant {
        ButtonVariant::Joystick => {
            draw_joystick_d2d(target, btn, text_format, brush_joystick_bg, brush_joystick_knob, brush_text, is_editing, is_selected, is_hover, edit_border, edit_hover, edit_normal);
        }
        ButtonVariant::Trigger => {
            draw_trigger_d2d(target, btn, text_format, brush_normal, brush_trigger_fill, brush_text, is_editing, is_selected, is_hover, edit_border, edit_hover, edit_normal);
        }
        _ => {
            draw_button_d2d(target, btn, text_format, brush_normal, brush_pressed, brush_text, is_editing, is_selected, is_hover, edit_border, edit_hover, edit_normal);
        }
    }
}

/// 绘制运行态或菜单态下的虚拟按键层。
unsafe fn draw_running_d2d(
    target: &ID2D1RenderTarget,
    state: &AppState,
    brush_normal: &ID2D1SolidColorBrush,
    brush_pressed: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    brush_joystick_bg: &ID2D1SolidColorBrush,
    brush_joystick_knob: &ID2D1SolidColorBrush,
    brush_trigger_fill: &ID2D1SolidColorBrush,
) {
    let text_format = state.buffer.text_format.as_ref();

    for btn in &state.buttons {
        draw_vbutton_d2d(
            target, btn, text_format,
            brush_normal, brush_pressed, brush_text,
            brush_joystick_bg, brush_joystick_knob, brush_trigger_fill,
            false, false, false,
            brush_normal, brush_normal, brush_normal,
        );
    }
}

/// 绘制编辑态下的按键、高亮和组合包围框。
unsafe fn draw_editing_d2d(
    target: &ID2D1RenderTarget,
    state: &AppState,
    brush_normal: &ID2D1SolidColorBrush,
    brush_pressed: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
    brush_joystick_bg: &ID2D1SolidColorBrush,
    brush_joystick_knob: &ID2D1SolidColorBrush,
    brush_trigger_fill: &ID2D1SolidColorBrush,
    edit_border: &ID2D1SolidColorBrush,
    edit_hover: &ID2D1SolidColorBrush,
    edit_normal: &ID2D1SolidColorBrush,
    group_border: &ID2D1SolidColorBrush,
    group_selected: &ID2D1SolidColorBrush,
) {
    let text_format = state.buffer.text_format.as_ref();

    let mut groups = std::collections::HashMap::new();
    for btn in &state.buttons {
        if btn.group_id != 0 {
            groups.entry(btn.group_id).or_insert(Vec::new()).push(btn.rect);
        }
    }

    for (gid, rects) in groups {
        if rects.is_empty() { continue; }
        let l = rects.iter().map(|r| r.left).min().unwrap_or(0);
        let t = rects.iter().map(|r| r.top).min().unwrap_or(0);
        let r = rects.iter().map(|r| r.right).max().unwrap_or(0);
        let b = rects.iter().map(|r| r.bottom).max().unwrap_or(0);

        let bound = Common::D2D_RECT_F {
            left: (l - 5) as f32,
            top: (t - 5) as f32,
            right: (r + 5) as f32,
            bottom: (b + 5) as f32,
        };

        // 仅在组合包围框内填充极低透明度，用于拦截编辑态点击。
        let bg_brush = target.CreateSolidColorBrush(&Common::D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.005 }, None).unwrap();
        target.FillRectangle(&bound, &bg_brush);

        let is_selected_group = state.edit_selected_group_id == Some(gid);
        let border_brush = if is_selected_group { group_selected } else { group_border };
        border_brush.SetOpacity(0.8);
        target.DrawRectangle(&bound, border_brush, if is_selected_group { 2.0 } else { 1.0 }, None);
    }

    for (idx, btn) in state.buttons.iter().enumerate() {
        let mut is_selected = false;
        let mut current_border = edit_border;

        if state.combo_select_mode.is_some() {
            if state.combo_select_temp.contains(&idx) {
                is_selected = true;
                current_border = group_selected;
            }
        } else {
            is_selected = if let ProgramMode::ButtonDetail(i) = state.mode { i == idx } else { false };
        }

        let is_hover = PtInRect(&btn.rect.into(), state.last_mouse_pos).as_bool();
        let is_group_selected = state.group_selected.map_or(false, |g| g == btn.group);
        
        draw_vbutton_d2d(
            target, btn, text_format,
            brush_normal, brush_pressed, brush_text,
            brush_joystick_bg, brush_joystick_knob, brush_trigger_fill,
            true, is_selected || is_group_selected, is_hover,
            current_border, edit_hover, edit_normal,
        );
    }
}

/// 绘制悬浮全键盘窗口及其内部按键。
unsafe fn draw_osk_d2d(
    target: &ID2D1RenderTarget,
    state: &AppState,
    brush_bg: &ID2D1SolidColorBrush,
    brush_title: &ID2D1SolidColorBrush,
    brush_normal: &ID2D1SolidColorBrush,
    brush_pressed: &ID2D1SolidColorBrush,
    brush_text: &ID2D1SolidColorBrush,
) {
    let text_format = state.buffer.text_format.as_ref();

    let osk_rect = Common::D2D_RECT_F {
        left: state.osk_rect.left as f32,
        top: state.osk_rect.top as f32,
        right: state.osk_rect.right as f32,
        bottom: state.osk_rect.bottom as f32,
    };
    let osk_rounded = D2D1_ROUNDED_RECT {
        rect: osk_rect,
        radiusX: 10.0,
        radiusY: 10.0,
    };
    brush_bg.SetOpacity(0.9);
    target.FillRoundedRectangle(&osk_rounded, brush_bg);

    let close_btn_rect = Common::D2D_RECT_F {
        left: (state.osk_rect.right - 40) as f32,
        top: state.osk_rect.top as f32,
        right: state.osk_rect.right as f32,
        bottom: (state.osk_rect.top + 30) as f32,
    };
    let brush_close_bg = target.CreateSolidColorBrush(&Common::D2D1_COLOR_F { r: 0.8, g: 0.2, b: 0.2, a: 0.8 }, None).unwrap();
    target.FillRectangle(&close_btn_rect, &brush_close_bg);
    
    if let Some(tf) = text_format {
        let text: Vec<u16> = "×".encode_utf16().collect();
        brush_text.SetOpacity(1.0);
        draw_text_d2d(target, "×", &close_btn_rect, tf, brush_text);
    }

    let title_rect = Common::D2D_RECT_F {
        left: state.osk_rect.left as f32,
        top: state.osk_rect.top as f32,
        right: (state.osk_rect.right - 40) as f32,
        bottom: (state.osk_rect.top + 50) as f32,
    };
    let title_rounded = D2D1_ROUNDED_RECT {
        rect: title_rect,
        radiusX: 10.0,
        radiusY: 10.0,
    };
    brush_title.SetOpacity(1.0);
    target.FillRoundedRectangle(&title_rounded, brush_title);

    if let Some(tf) = text_format {
        brush_text.SetOpacity(1.0);
        draw_text_d2d(target, "全键盘 (拖动标题栏)", &title_rect, tf, brush_text);
    }

    for btn in &state.osk_buttons {
        let btn_rect = Common::D2D_RECT_F {
            left: (state.osk_rect.left + btn.rect.left) as f32,
            top: (state.osk_rect.top + btn.rect.top) as f32,
            right: (state.osk_rect.left + btn.rect.right) as f32,
            bottom: (state.osk_rect.top + btn.rect.bottom) as f32,
        };

        let btn_alpha = btn.opacity as f32 / 255.0;
        let current_brush = if btn.is_pressed { brush_pressed } else { brush_normal };
        current_brush.SetOpacity(btn_alpha);

        let btn_rounded = D2D1_ROUNDED_RECT {
            rect: btn_rect,
            radiusX: 5.0,
            radiusY: 5.0,
        };
        target.FillRoundedRectangle(&btn_rounded, current_brush);

        if let Some(tf) = text_format {
            brush_text.SetOpacity(btn_alpha);
            draw_text_d2d(target, &btn.label, &btn_rect, tf, brush_text);
        }
    }
}
