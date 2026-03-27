//! 文件作用：处理鼠标、触控和侧边栏选择事件，并驱动虚拟按键状态变化。

use crate::app_state::*;
use crate::input::handler::*;
use crate::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{PtInRect, InvalidateRect};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use crate::app_state::TouchRecord;

const TOUCH_OSK_DRAG_SENTINEL: usize = 9999;
const TOUCH_OSK_KEY_SENTINEL_BASE: usize = 10000;

fn system_osk_target() -> String {
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".to_string());
    let sysnative = format!("{}\\sysnative\\osk.exe", windir);
    if std::path::Path::new(&sysnative).exists() {
        sysnative
    } else {
        format!("{}\\System32\\osk.exe", windir)
    }
}

pub unsafe fn is_system_osk_open() -> bool {
    FindWindowW(w!("OSKMainClass"), PCWSTR::null())
        .map(|hwnd| IsWindow(hwnd).as_bool())
        .unwrap_or(false)
}

pub unsafe fn open_system_osk() {
    let target = system_osk_target();
    let mut wide_target: Vec<u16> = target.encode_utf16().collect();
    wide_target.push(0);
    let _ = ShellExecuteW(None, w!("open"), PCWSTR(wide_target.as_ptr()), None, None, SW_SHOW);
}

pub unsafe fn close_system_osk() {
    if let Ok(osk_hwnd) = FindWindowW(w!("OSKMainClass"), PCWSTR::null()) {
        if IsWindow(osk_hwnd).as_bool() {
            let _ = PostMessageW(osk_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

pub unsafe fn close_virtual_osk(state: &mut AppState) {
    state.osk_visible = false;
    state.is_dragging_osk = false;
    state.osk_target_text = None;
    for btn in &mut state.osk_buttons {
        if btn.is_pressed {
            let focus_hwnd = GetFocus();
            if !focus_hwnd.is_invalid() {
                let scan = MapVirtualKeyW(btn.key_code as u32, MAPVK_VK_TO_VSC);
                let _ = PostMessageW(
                    focus_hwnd,
                    WM_KEYUP,
                    WPARAM(btn.key_code as usize),
                    LPARAM((1 | (scan << 16) | (1 << 30) | (1 << 31)) as isize),
                );
            } else {
                simulate_key(VIRTUAL_KEY(btn.key_code), false);
            }
        }
        btn.is_pressed = false;
    }
}

pub unsafe fn close_all_osk(state: &mut AppState) {
    close_virtual_osk(state);
    close_system_osk();
}

pub unsafe fn toggle_osk(state: &mut AppState) {
    if state.use_system_osk {
        if is_system_osk_open() {
            close_system_osk();
        } else {
            open_system_osk();
        }
    } else if state.osk_visible {
        close_virtual_osk(state);
    } else {
        state.osk_visible = true;
    }
}

/// 关闭悬浮键盘，并将焦点切回其父窗口。
unsafe fn close_osk_and_defocus(state: &mut AppState) {
    close_virtual_osk(state);
    
    let focus_hwnd = GetFocus();
    if !focus_hwnd.is_invalid() {
        if let Ok(parent) = GetParent(focus_hwnd) {
            if !parent.is_invalid() {
                let _ = SetFocus(parent);
            }
        }
    }
}

/// 计算编辑态按键拖动时的吸附位置。
///
/// `current_idx` 用于跳过当前按键，`snap_dist` 表示允许吸附的最大像素距离。
fn calculate_snap(current_idx: usize, current_rect: RECT, buttons: &[VirtualButton], snap_dist: i32) -> (i32, i32) {
    let w = current_rect.right - current_rect.left; let h = current_rect.bottom - current_rect.top;
    let mut new_l = current_rect.left; let mut new_t = current_rect.top;
    let mut min_diff_x = snap_dist + 1; let mut target_x = new_l;
    let mut min_diff_y = snap_dist + 1; let mut target_y = new_t;

    for (i, other) in buttons.iter().enumerate() {
        if i == current_idx { continue; }
        let o_r: RECT = other.rect.into();
        for &(edge, new_edge, is_x) in &[
            (o_r.left, new_l, true), (o_r.right, new_l, true), (o_r.left, new_l + w, true), (o_r.right, new_l + w, true),
            (o_r.top, new_t, false), (o_r.bottom, new_t, false), (o_r.top, new_t + h, false), (o_r.bottom, new_t + h, false),
        ] {
            let diff = (new_edge - edge).abs();
            if is_x { if diff < min_diff_x { min_diff_x = diff; target_x = if new_edge == new_l { edge } else { edge - w }; } } 
            else { if diff < min_diff_y { min_diff_y = diff; target_y = if new_edge == new_t { edge } else { edge - h }; } }
        }
    }
    if min_diff_x <= snap_dist { new_l = target_x; }
    if min_diff_y <= snap_dist { new_t = target_y; }
    (new_l, new_t)
}

/// 计算一个组合内所有按键的外包矩形。
fn get_group_bounding_rect(state: &AppState, group_id: u32) -> Option<RECT> {
    if group_id == 0 { return None; }
    let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut found = false;
    for btn in &state.buttons {
        if btn.group_id == group_id {
            let r: RECT = btn.rect.into();
            left = left.min(r.left); top = top.min(r.top); right = right.max(r.right); bottom = bottom.max(r.bottom); found = true;
        }
    }
    if found { Some(RECT { left, top, right, bottom }) } else { None }
}

/// 处理鼠标左键按下时的按键命中、拖拽和编辑态选择逻辑。
pub unsafe fn on_lbutton_down(state: &mut AppState, hwnd: HWND, pt: POINT, _is_double_click: bool) {
    state.last_mouse_pos = pt;

    if state.osk_visible {
        let shrink_btn = RECT { left: state.osk_rect.right - 40, top: state.osk_rect.top, right: state.osk_rect.right, bottom: state.osk_rect.top + 30 };
        if PtInRect(&shrink_btn, pt).as_bool() { close_osk_and_defocus(state); return; }

        let title_rect = RECT { left: state.osk_rect.left, top: state.osk_rect.top, right: state.osk_rect.right, bottom: state.osk_rect.top + 50 };
        if PtInRect(&title_rect, pt).as_bool() {
            SetCapture(hwnd); state.is_dragging_osk = true; state.osk_drag_offset = POINT { x: pt.x - state.osk_rect.left, y: pt.y - state.osk_rect.top };
            return;
        }

        if PtInRect(&state.osk_rect, pt).as_bool() {
            for btn in &mut state.osk_buttons {
                let r: RECT = btn.rect.into(); let abs_r = RECT { left: state.osk_rect.left + r.left, top: state.osk_rect.top + r.top, right: state.osk_rect.left + r.right, bottom: state.osk_rect.top + r.bottom };
                if PtInRect(&abs_r, pt).as_bool() {
                    btn.is_pressed = true; let focus_hwnd = GetFocus();
                    if !focus_hwnd.is_invalid() {
                        let scan = MapVirtualKeyW(btn.key_code as u32, MAPVK_VK_TO_VSC); let _ = PostMessageW(focus_hwnd, WM_KEYDOWN, WPARAM(btn.key_code as usize), LPARAM((1 | (scan << 16)) as isize));
                    } else { simulate_key(VIRTUAL_KEY(btn.key_code), true); }
                    
                    if btn.key_code == VK_RETURN.0 {
                        close_osk_and_defocus(state);
                    }
                    return; 
                }
            }
            return;
        } else {
            close_osk_and_defocus(state);
            return;
        }
    }

    SetCapture(hwnd);

    if state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu {
        for (i, btn) in state.buttons.iter_mut().enumerate().rev() {
            if PtInRect(&btn.rect.into(), pt).as_bool() {
                if btn.variant == ButtonVariant::Joystick || btn.variant == ButtonVariant::Trigger {
                    btn.is_pressed = true; state.touchpad_active_button = Some(i); state.touchpad_last_pos = pt; return;
                }

                if btn.group == 5 {
                    btn.is_pressed = true;
                    state.group_drag_start_pt = pt;
                } else if btn.variant == ButtonVariant::OSKToggle {
                    btn.is_pressed = true;
                } else if btn.group == 1 { 
                    btn.is_pressed = !btn.is_pressed; simulate_key(VIRTUAL_KEY(btn.key_code), btn.is_pressed); 
                } else if btn.group == 3 { 
                    btn.is_pressed = true; state.touchpad_active_button = Some(i); state.touchpad_last_pos = pt; 
                } else if btn.group == 4 {
                    btn.is_pressed = true; state.touchpad_active_button = Some(i); state.touchpad_last_pos = pt; 
                    let screen_w = GetSystemMetrics(SM_CXSCREEN);
                    let screen_h = GetSystemMetrics(SM_CYSCREEN);
                    crate::input::handler::simulate_mouse_absolute(pt.x, pt.y, screen_w, screen_h);
                    simulate_key(VIRTUAL_KEY(btn.key_code), true);
                } else if btn.group == 6 {
                    btn.is_pressed = true; state.touchpad_active_button = Some(i);
                } else { 
                    btn.is_pressed = true; simulate_key(VIRTUAL_KEY(btn.key_code), true); 
                }
                state.dragging_button_index = Some(i);
                
                let r: RECT = btn.rect.into();
                state.drag_offset = POINT { x: pt.x - r.left, y: pt.y - r.top };
                
                if state.use_virtual_gamepad {
                    crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
                }
                break;
            }
        }
    } else if matches!(state.mode, ProgramMode::Editing | ProgramMode::ButtonDetail(_) | ProgramMode::GroupDetail(_)) {
        if state.combo_select_mode.is_some() {
            let mut hit_idx = None;
            for (i, btn) in state.buttons.iter().enumerate().rev() {
                if PtInRect(&btn.rect.into(), pt).as_bool() { hit_idx = Some(i); break; }
            }
            if let Some(idx) = hit_idx {
                if let Some(pos) = state.combo_select_temp.iter().position(|&x| x == idx) {
                    state.combo_select_temp.remove(pos);
                } else {
                    state.combo_select_temp.push(idx);
                }
                let _ = InvalidateRect(hwnd, None, TRUE);
            }
            return;
        }

        let mut hit_individual = None;
        for (i, btn) in state.buttons.iter().enumerate().rev() {
            if PtInRect(&btn.rect.into(), pt).as_bool() { hit_individual = Some(i); break; }
        }

        if let Some(idx) = hit_individual {
            state.dragging_button_index = Some(idx);
            let r: RECT = state.buttons[idx].rect.into(); state.drag_offset = POINT { x: pt.x - r.left, y: pt.y - r.top };
            state.active_group_drag_id = if state.buttons[idx].group_id != 0 { Some(state.buttons[idx].group_id) } else { None };
            if state.active_group_drag_id.is_some() { state.group_drag_start_pt = pt; }
            if !matches!(state.mode, ProgramMode::ButtonDetail(i) if i == idx) {
                state.mode = ProgramMode::ButtonDetail(idx);
                crate::ui::panels::show_button_property_window(hwnd, idx, state);
            }
            return;
        }

        let mut group_ids: Vec<u32> = state.buttons.iter().map(|b| b.group_id).filter(|&id| id != 0).collect();
        group_ids.sort(); group_ids.dedup();

        for gid in group_ids {
            if let Some(bbox) = get_group_bounding_rect(state, gid) {
                let group_rect = RECT { left: bbox.left - 5, top: bbox.top - 5, right: bbox.right + 5, bottom: bbox.bottom + 5 };
                if PtInRect(&group_rect, pt).as_bool() {
                    state.active_group_drag_id = Some(gid); state.group_drag_start_pt = pt; state.edit_selected_group_id = Some(gid);
                    state.mode = ProgramMode::GroupDetail(gid); state.dragging_button_index = None;
                    let btn_cat = state.buttons.iter().find(|b| b.group_id == gid).map(|b| b.combo_category as usize).unwrap_or(0);
                    let mut same_cat_groups: Vec<u32> = state.buttons.iter().filter(|b| b.combo_category == gid as i32 || (b.combo_category >= 0 && b.group_id == gid)).map(|b| b.group_id).collect();
                    same_cat_groups.sort(); same_cat_groups.dedup(); let inst_idx = same_cat_groups.iter().position(|&g| g == gid).unwrap_or(0);
                    crate::ui::panels::show_combo_instance_window(hwnd, btn_cat, inst_idx, gid, state);
                    return;
                }
            }
        }
        state.mode = ProgramMode::Editing; state.edit_selected_group_id = None; state.dragging_button_index = None; state.active_group_drag_id = None;
        let old_prop = GetPropW(hwnd, w!("PropertyHwnd"));
        if !old_prop.is_invalid() { DestroyWindow(HWND(old_prop.0 as *mut _)); RemovePropW(hwnd, w!("PropertyHwnd")); }
    }
}

/// 处理鼠标左键抬起后的释放、点击确认和状态复位。
pub unsafe fn on_lbutton_up(state: &mut AppState) {
    let _ = ReleaseCapture();
    state.active_group_drag_id = None; state.is_dragging_osk = false;

    if state.osk_visible {
        for btn in &mut state.osk_buttons {
            if btn.is_pressed {
                btn.is_pressed = false; let focus_hwnd = GetFocus();
                if !focus_hwnd.is_invalid() {
                    let scan = MapVirtualKeyW(btn.key_code as u32, MAPVK_VK_TO_VSC); let _ = PostMessageW(focus_hwnd, WM_KEYUP, WPARAM(btn.key_code as usize), LPARAM((1 | (scan << 16) | (1 << 30) | (1 << 31)) as isize));
                } else { simulate_key(VIRTUAL_KEY(btn.key_code), false); }
            }
        }
    }

    if let Some(idx) = state.touchpad_active_button {
        if state.buttons[idx].variant == ButtonVariant::Joystick || state.buttons[idx].variant == ButtonVariant::Trigger { state.buttons[idx].joystick_val = (0.0, 0.0); }
    }

    if state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu {
        let mut should_toggle_osk = false;
        for btn in state.buttons.iter_mut() {
            if btn.group == 5 || btn.variant == ButtonVariant::OSKToggle {
                if btn.is_pressed {
                    btn.is_pressed = false;
                    // 仅在确认是点击而非拖拽后，才执行自由按键或键盘开关逻辑。
                    if btn.variant == ButtonVariant::OSKToggle {
                        should_toggle_osk = true;
                    } else {
                        // 自由按键在抬起时触发一次短促点击。
                        let key_code = btn.key_code;
                        std::thread::spawn(move || {
                            unsafe {
                                simulate_key(VIRTUAL_KEY(key_code), true);
                                std::thread::sleep(std::time::Duration::from_millis(30));
                                simulate_key(VIRTUAL_KEY(key_code), false);
                            }
                        });
                    }
                }
            } else if btn.group == 4 && btn.is_pressed {
                btn.is_pressed = false; 
                simulate_key(VIRTUAL_KEY(btn.key_code), false);
            } else if btn.group != 1 && btn.is_pressed && btn.variant == ButtonVariant::Normal { 
                btn.is_pressed = false; simulate_key(VIRTUAL_KEY(btn.key_code), false); 
            } else if btn.variant == ButtonVariant::Joystick || btn.variant == ButtonVariant::Trigger { 
                btn.is_pressed = false; 
            }
        }
        if should_toggle_osk {
            toggle_osk(state);
        }
        state.dragging_button_index = None; state.touchpad_active_button = None;
        
        if state.use_virtual_gamepad {
            crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
        }
    } else { state.dragging_button_index = None; }
}

/// 处理鼠标移动时的拖拽、滑动触发和摇杆更新。
pub unsafe fn on_mouse_move(state: &mut AppState, pt: POINT) -> bool {
    let mut need_redraw = false;
    if pt.x == state.last_mouse_pos.x && pt.y == state.last_mouse_pos.y { return false; }

    if state.is_dragging_osk {
        let (w, h) = (state.osk_rect.right - state.osk_rect.left, state.osk_rect.bottom - state.osk_rect.top);
        state.osk_rect.left = pt.x - state.osk_drag_offset.x; state.osk_rect.top = pt.y - state.osk_drag_offset.y; state.osk_rect.right = state.osk_rect.left + w; state.osk_rect.bottom = state.osk_rect.top + h; need_redraw = true;
    } else if let Some(idx) = state.dragging_button_index {
        if matches!(state.mode, ProgramMode::Editing | ProgramMode::ButtonDetail(_)) {
            let (w, h) = (state.buttons[idx].rect.right - state.buttons[idx].rect.left, state.buttons[idx].rect.bottom - state.buttons[idx].rect.top);
            let (new_l, new_t) = (pt.x - state.drag_offset.x, pt.y - state.drag_offset.y);
            let (sl, st) = calculate_snap(idx, RECT { left: new_l, top: new_t, right: new_l + w, bottom: new_t + h }, &state.buttons, 10);
            state.buttons[idx].rect = SerializableRect { left: sl, top: st, right: sl + w, bottom: st + h }; need_redraw = true;
        } else if state.buttons[idx].group == 5 && (state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu) {
            // 自由按键移动距离过大时，按拖拽处理而不是点击。
            let dx = pt.x - state.group_drag_start_pt.x;
            let dy = pt.y - state.group_drag_start_pt.y;
            if dx.abs() > 10 || dy.abs() > 10 {
                state.buttons[idx].is_pressed = false;
            }

            let (w, h) = (state.buttons[idx].rect.right - state.buttons[idx].rect.left, state.buttons[idx].rect.bottom - state.buttons[idx].rect.top);
            let (new_l, new_t) = (pt.x - state.drag_offset.x, pt.y - state.drag_offset.y);
            state.buttons[idx].rect = SerializableRect { left: new_l, top: new_t, right: new_l + w, bottom: new_t + h }; 
            need_redraw = true;
        }
    } else if let Some(gid) = state.active_group_drag_id {
        let (dx, dy) = (pt.x - state.group_drag_start_pt.x, pt.y - state.group_drag_start_pt.y);
        for btn in state.buttons.iter_mut().filter(|b| b.group_id == gid) { btn.rect.left += dx; btn.rect.right += dx; btn.rect.top += dy; btn.rect.bottom += dy; }
        state.group_drag_start_pt = pt; need_redraw = true;
    }

    if state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu {
        let mut current_hover_idx = None;
        for (i, btn) in state.buttons.iter().enumerate() { if PtInRect(&btn.rect.into(), pt).as_bool() { current_hover_idx = Some(i); break; } }
        if GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 {
            // 自由按键拖拽期间，不切换滑动触发目标。
            let is_dragging_free = state.dragging_button_index.map(|idx| state.buttons[idx].group == 5).unwrap_or(false);
            
            if !is_dragging_free && current_hover_idx != state.dragging_button_index {
                if let Some(oidx) = state.dragging_button_index { if state.buttons[oidx].group == 2 { simulate_key(VIRTUAL_KEY(state.buttons[oidx].key_code), false); state.buttons[oidx].is_pressed = false; } }
                if let Some(nidx) = current_hover_idx { if state.buttons[nidx].group == 2 { simulate_key(VIRTUAL_KEY(state.buttons[nidx].key_code), true); state.buttons[nidx].is_pressed = true; } }
                state.dragging_button_index = current_hover_idx; need_redraw = true;
            }
        }
    }

    if let Some(idx) = state.touchpad_active_button {
        let btn = &mut state.buttons[idx];
        if btn.is_pressed {
            if btn.group == 3 && (state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu) {
                let (dx, dy) = (pt.x - state.touchpad_last_pos.x, pt.y - state.touchpad_last_pos.y);
                if (dx != 0 || dy != 0) && dx.abs() < 200 && dy.abs() < 200 {
                    let sens = btn.sensitivity;
                    simulate_mouse_move_relative((dx as f32 * sens) as i32, (dy as f32 * sens) as i32);
                }
                state.touchpad_last_pos = pt;
            } else if btn.group == 4 && (state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu) {
                let screen_w = GetSystemMetrics(SM_CXSCREEN);
                let screen_h = GetSystemMetrics(SM_CYSCREEN);
                crate::input::handler::simulate_mouse_absolute(pt.x, pt.y, screen_w, screen_h);
                state.touchpad_last_pos = pt;
            } else if btn.variant == ButtonVariant::Joystick {
                let cx = state.touchpad_last_pos.x;
                let cy = state.touchpad_last_pos.y;
                let (dx, dy) = (pt.x - cx, pt.y - cy);
                let dist = ((dx * dx + dy * dy) as f32).sqrt();
                let max_dist = (btn.rect.right - btn.rect.left) as f32 / if state.mode == ProgramMode::Running { 2.0 } else { 1.0 };
                btn.joystick_val = if dist > max_dist { 
                    (dx as f32 / dist, dy as f32 / dist) 
                } else { 
                    (dx as f32 / max_dist, dy as f32 / max_dist) 
                }; 
                need_redraw = true;
            } else if btn.variant == ButtonVariant::Trigger {
                let h = btn.rect.bottom - btn.rect.top; btn.joystick_val = (0.0, ((btn.rect.bottom - pt.y) as f32 / h as f32).clamp(0.0, 1.0)); need_redraw = true;
            }
        }
    }
    
    if need_redraw && (state.mode == ProgramMode::Running || state.mode == ProgramMode::Menu) {
        if state.use_virtual_gamepad {
            crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
        }
    }
    
    state.last_mouse_pos = pt; need_redraw
}

/// 响应侧边栏节点选择，并切换到对应按钮或分组视图。
pub fn on_tree_item_selected(state: &mut AppState, item_data: isize, hwnd: HWND) -> bool {
    state.combo_select_mode = None;
    state.combo_select_temp.clear();
    
    if item_data == 0 { return false; }
    if item_data > 0 {
        state.group_selected = Some((item_data - 1) as u8); 
        false
    } else {
        let btn_idx = (-item_data - 1) as usize; 
        
        if let ProgramMode::ButtonDetail(i) = state.mode {
            if i == btn_idx { return true; }
        }
        
        state.mode = ProgramMode::ButtonDetail(btn_idx);
        unsafe { crate::ui::panels::show_button_property_window(hwnd, btn_idx, state); } 
        true
    }
}

/// 处理触控按下时的命中检测和状态记录。
pub unsafe fn on_pointer_down(state: &mut AppState, pointer_id: u32, pt: POINT) {
    if state.osk_visible {
        let shrink_btn = RECT { left: state.osk_rect.right - 40, top: state.osk_rect.top, right: state.osk_rect.right, bottom: state.osk_rect.top + 30 };
        if PtInRect(&shrink_btn, pt).as_bool() { close_osk_and_defocus(state); return; }

        let title_rect = RECT { left: state.osk_rect.left, top: state.osk_rect.top, right: state.osk_rect.right, bottom: state.osk_rect.top + 50 };
        if PtInRect(&title_rect, pt).as_bool() {
            state.is_dragging_osk = true;
            state.osk_drag_offset = POINT { x: pt.x - state.osk_rect.left, y: pt.y - state.osk_rect.top };
            state.active_touches.insert(pointer_id, TouchRecord { btn_idx: TOUCH_OSK_DRAG_SENTINEL, last_pos: pt, start_pos: pt, drag_offset: POINT::default() });
            return;
        }

        if PtInRect(&state.osk_rect, pt).as_bool() {
            for btn in &mut state.osk_buttons {
                let r: RECT = btn.rect.into(); let abs_r = RECT { left: state.osk_rect.left + r.left, top: state.osk_rect.top + r.top, right: state.osk_rect.left + r.right, bottom: state.osk_rect.top + r.bottom };
                if PtInRect(&abs_r, pt).as_bool() {
                    btn.is_pressed = true; let focus_hwnd = GetFocus();
                    if !focus_hwnd.is_invalid() {
                        let scan = MapVirtualKeyW(btn.key_code as u32, MAPVK_VK_TO_VSC); let _ = PostMessageW(focus_hwnd, WM_KEYDOWN, WPARAM(btn.key_code as usize), LPARAM((1 | (scan << 16)) as isize));
                    } else { simulate_key(VIRTUAL_KEY(btn.key_code), true); }
                    state.active_touches.insert(pointer_id, TouchRecord { btn_idx: TOUCH_OSK_KEY_SENTINEL_BASE + btn.key_code as usize, last_pos: pt, start_pos: pt, drag_offset: POINT::default() });
                    
                    if btn.key_code == VK_RETURN.0 {
                        close_osk_and_defocus(state);
                    }
                    return; 
                }
            }
            return;
        } else {
            close_osk_and_defocus(state);
            return;
        }
    }

    for (i, btn) in state.buttons.iter_mut().enumerate().rev() {
        if PtInRect(&btn.rect.into(), pt).as_bool() {
            let r: RECT = btn.rect.into();
            let drag_offset = POINT { x: pt.x - r.left, y: pt.y - r.top };

            if btn.variant == ButtonVariant::Joystick || btn.variant == ButtonVariant::Trigger {
                btn.is_pressed = true; 
                if btn.group == 6 { state.touchpad_active_button = Some(i); }
            } else if btn.group == 5 {
                btn.is_pressed = true;
            } else if btn.variant == ButtonVariant::OSKToggle {
                btn.is_pressed = true;
            } else if btn.group == 1 { 
                btn.is_pressed = !btn.is_pressed; simulate_key(VIRTUAL_KEY(btn.key_code), btn.is_pressed); 
            } else if btn.group == 3 { 
                btn.is_pressed = true; 
            } else if btn.group == 4 {
                btn.is_pressed = true; 
                let screen_w = GetSystemMetrics(SM_CXSCREEN); let screen_h = GetSystemMetrics(SM_CYSCREEN);
                crate::input::handler::simulate_mouse_absolute(pt.x, pt.y, screen_w, screen_h);
                simulate_key(VIRTUAL_KEY(btn.key_code), true);
            } else if btn.group == 6 {
                btn.is_pressed = true; state.touchpad_active_button = Some(i);
            } else { 
                btn.is_pressed = true; simulate_key(VIRTUAL_KEY(btn.key_code), true); 
            }
            
            state.active_touches.insert(pointer_id, TouchRecord { btn_idx: i, last_pos: pt, start_pos: pt, drag_offset });
            
            if state.use_virtual_gamepad { crate::input::vigem_wrapper::sync_gamepad(&state.buttons); }
            break;
        }
    }
}

/// 处理触控移动时的拖拽、滑动和模拟量更新。
pub unsafe fn on_pointer_update(state: &mut AppState, pointer_id: u32, pt: POINT) -> bool {
    let mut need_redraw = false;

    if let Some(mut touch) = state.active_touches.get_mut(&pointer_id).copied() {
        if touch.btn_idx == TOUCH_OSK_DRAG_SENTINEL {
            if state.is_dragging_osk {
                let (w, h) = (state.osk_rect.right - state.osk_rect.left, state.osk_rect.bottom - state.osk_rect.top);
                state.osk_rect.left = pt.x - state.osk_drag_offset.x; state.osk_rect.top = pt.y - state.osk_drag_offset.y; 
                state.osk_rect.right = state.osk_rect.left + w; state.osk_rect.bottom = state.osk_rect.top + h; need_redraw = true;
            }
        } else if touch.btn_idx < TOUCH_OSK_KEY_SENTINEL_BASE {
            let idx = touch.btn_idx;
            
            if state.buttons[idx].group == 5 {
                let dx = pt.x - touch.start_pos.x; let dy = pt.y - touch.start_pos.y;
                if dx.abs() > 10 || dy.abs() > 10 { state.buttons[idx].is_pressed = false; }
                let (w, h) = (state.buttons[idx].rect.right - state.buttons[idx].rect.left, state.buttons[idx].rect.bottom - state.buttons[idx].rect.top);
                let new_l = pt.x - touch.drag_offset.x; let new_t = pt.y - touch.drag_offset.y;
                state.buttons[idx].rect = crate::app_state::SerializableRect { left: new_l, top: new_t, right: new_l + w, bottom: new_t + h }; 
                need_redraw = true;
            }

            let current_btn = &mut state.buttons[idx];
            if current_btn.is_pressed {
                if current_btn.group == 3 {
                    let (dx, dy) = (pt.x - touch.last_pos.x, pt.y - touch.last_pos.y);
                    if (dx != 0 || dy != 0) && dx.abs() < 200 && dy.abs() < 200 {
                        let sens = current_btn.sensitivity;
                        simulate_mouse_move_relative((dx as f32 * sens) as i32, (dy as f32 * sens) as i32);
                    }
                } else if current_btn.group == 4 {
                    let screen_w = GetSystemMetrics(SM_CXSCREEN); let screen_h = GetSystemMetrics(SM_CYSCREEN);
                    crate::input::handler::simulate_mouse_absolute(pt.x, pt.y, screen_w, screen_h);
                } else if current_btn.variant == ButtonVariant::Joystick {
                    let cx = touch.start_pos.x; let cy = touch.start_pos.y;
                    let (dx, dy) = (pt.x - cx, pt.y - cy);
                    let dist = ((dx * dx + dy * dy) as f32).sqrt();
                    let max_dist = (current_btn.rect.right - current_btn.rect.left) as f32 / 2.0;
                    current_btn.joystick_val = if dist > max_dist { (dx as f32 / dist, dy as f32 / dist) } else { (dx as f32 / max_dist, dy as f32 / max_dist) }; 
                    need_redraw = true;
                } else if current_btn.variant == ButtonVariant::Trigger {
                    let h = current_btn.rect.bottom - current_btn.rect.top; 
                    current_btn.joystick_val = (0.0, ((current_btn.rect.bottom - pt.y) as f32 / h as f32).clamp(0.0, 1.0)); 
                    need_redraw = true;
                }
            }

            if state.buttons[idx].group == 2 {
                let mut current_hover_idx = None;
                for (i, b) in state.buttons.iter().enumerate() { 
                    if PtInRect(&b.rect.into(), pt).as_bool() { current_hover_idx = Some(i); break; } 
                }
                if let Some(nidx) = current_hover_idx {
                    if nidx != idx && state.buttons[nidx].group == 2 {
                        simulate_key(VIRTUAL_KEY(state.buttons[idx].key_code), false); state.buttons[idx].is_pressed = false; 
                        simulate_key(VIRTUAL_KEY(state.buttons[nidx].key_code), true); state.buttons[nidx].is_pressed = true; 
                        touch.btn_idx = nidx;
                        need_redraw = true;
                    }
                }
            }
        }
        
        touch.last_pos = pt;
        state.active_touches.insert(pointer_id, touch);
        
        if need_redraw && state.use_virtual_gamepad {
            crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
        }
    }
    need_redraw
}

/// 处理触控抬起时的释放、点击确认和同步收尾。
pub unsafe fn on_pointer_up(state: &mut AppState, pointer_id: u32) {
    if let Some(touch) = state.active_touches.remove(&pointer_id) {
        if touch.btn_idx == TOUCH_OSK_DRAG_SENTINEL {
            state.is_dragging_osk = false;
        } else if touch.btn_idx >= TOUCH_OSK_KEY_SENTINEL_BASE {
            let key_code = (touch.btn_idx - TOUCH_OSK_KEY_SENTINEL_BASE) as u16;
            for btn in &mut state.osk_buttons {
                if btn.key_code == key_code {
                    btn.is_pressed = false; 
                    let focus_hwnd = GetFocus();
                    if !focus_hwnd.is_invalid() {
                        let scan = MapVirtualKeyW(btn.key_code as u32, MAPVK_VK_TO_VSC); 
                        let _ = PostMessageW(focus_hwnd, WM_KEYUP, WPARAM(btn.key_code as usize), LPARAM((1 | (scan << 16) | (1 << 30) | (1 << 31)) as isize));
                    } else { simulate_key(VIRTUAL_KEY(btn.key_code), false); }
                }
            }
        } else {
            let idx = touch.btn_idx;
            let mut should_toggle_osk = false;
            let btn = &mut state.buttons[idx];

            if btn.group == 5 || btn.variant == ButtonVariant::OSKToggle {
                if btn.is_pressed {
                    btn.is_pressed = false;
                    // 仅在确认是点击而非拖拽后，才执行自由按键或键盘开关逻辑。
                    if btn.variant == ButtonVariant::OSKToggle {
                        should_toggle_osk = true;
                    } else {
                        // 自由按键在抬起时触发一次短促点击。
                        let key_code = btn.key_code;
                        std::thread::spawn(move || {
                            unsafe {
                                simulate_key(VIRTUAL_KEY(key_code), true);
                                std::thread::sleep(std::time::Duration::from_millis(30));
                                simulate_key(VIRTUAL_KEY(key_code), false);
                            }
                        });
                    }
                }
            } else if btn.group == 4 && btn.is_pressed {
                btn.is_pressed = false; simulate_key(VIRTUAL_KEY(btn.key_code), false);
            } else if btn.group != 1 && btn.is_pressed && btn.variant == ButtonVariant::Normal { 
                btn.is_pressed = false; simulate_key(VIRTUAL_KEY(btn.key_code), false); 
            } else if btn.variant == ButtonVariant::Joystick || btn.variant == ButtonVariant::Trigger { 
                btn.is_pressed = false; btn.joystick_val = (0.0, 0.0);
            }
            
            if btn.group == 6 && state.touchpad_active_button == Some(idx) {
                state.touchpad_active_button = None;
            }
            if should_toggle_osk {
                toggle_osk(state);
            }

            if state.use_virtual_gamepad {
                crate::input::vigem_wrapper::sync_gamepad(&state.buttons);
            }
        }
    }
}
