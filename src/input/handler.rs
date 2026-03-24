//! 文件作用：封装键盘、鼠标与手柄映射后的输入注入逻辑。

use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub const INJECTED_INPUT_SIGNATURE: usize = 0xFFC3C3;

/// 将部分手柄虚拟键映射为对应的键盘按键。
fn map_gamepad_to_vk(vk: VIRTUAL_KEY) -> Option<VIRTUAL_KEY> {
    match vk {
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_A.0 => Some(VK_SPACE),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_B.0 => Some(VK_ESCAPE),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_X.0 => Some(VK_E),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_Y.0 => Some(VK_Q),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_LEFT_SHOULDER.0 => Some(VK_Q),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_RIGHT_SHOULDER.0 => Some(VK_E),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_DPAD_UP.0 => Some(VK_UP),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_DPAD_DOWN.0 => Some(VK_DOWN),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_DPAD_LEFT.0 => Some(VK_LEFT),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_DPAD_RIGHT.0 => Some(VK_RIGHT),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_MENU.0 => Some(VK_RETURN),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_VIEW.0 => Some(VK_TAB),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_LEFT_THUMBSTICK_BUTTON.0 => Some(VK_LSHIFT),
        VIRTUAL_KEY(u) if u == VK_GAMEPAD_RIGHT_THUMBSTICK_BUTTON.0 => Some(VK_LCONTROL),
        _ => None,
    }
}

/// 注入一次按键或鼠标按键的按下/抬起事件。
///
/// `vk` 表示目标虚拟键，`down` 为 `true` 时表示按下，`false` 表示抬起。
pub unsafe fn simulate_key(vk: VIRTUAL_KEY, down: bool) {
    let vk_u = vk.0 as u32;
    if vk_u >= 0x05E0 && vk_u <= 0x05FF {
        if let Some(mapped_vk) = map_gamepad_to_vk(vk) {
            simulate_key(mapped_vk, down);
            return;
        }
    }

    let code = vk.0 as u32;

    if code == VK_LBUTTON.0 as u32 || code == VK_RBUTTON.0 as u32 {
        let mut input = INPUT::default();
        input.r#type = INPUT_MOUSE;
        input.Anonymous.mi.dwFlags = match (code, down) {
            (c, true) if c == VK_LBUTTON.0 as u32 => MOUSEEVENTF_LEFTDOWN,
            (c, false) if c == VK_LBUTTON.0 as u32 => MOUSEEVENTF_LEFTUP,
            (c, true) if c == VK_RBUTTON.0 as u32 => MOUSEEVENTF_RIGHTDOWN,
            (_, false) => MOUSEEVENTF_RIGHTUP,
            _ => MOUSEEVENTF_LEFTUP,
        };
        input.Anonymous.mi.dwExtraInfo = INJECTED_INPUT_SIGNATURE;
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        return;
    }

    let scan_code = MapVirtualKeyW(code, MAPVK_VK_TO_VSC) as u16;
    
    let mut flags = KEYBD_EVENT_FLAGS(0);
    flags |= KEYEVENTF_SCANCODE;
    
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }

    let ext_keys = [
        VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0, VK_PRIOR.0, VK_NEXT.0,
        VK_END.0, VK_HOME.0, VK_INSERT.0, VK_DELETE.0, VK_NUMLOCK.0,
        VK_SNAPSHOT.0, VK_DIVIDE.0, VK_RMENU.0, VK_RCONTROL.0,
        VK_LWIN.0, VK_RWIN.0,
    ];
    if ext_keys.contains(&vk.0) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }

    let mut input = INPUT::default();
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous.ki.wVk = VIRTUAL_KEY(0);
    input.Anonymous.ki.wScan = scan_code;
    input.Anonymous.ki.dwFlags = flags;
    input.Anonymous.ki.time = 0;
    input.Anonymous.ki.dwExtraInfo = INJECTED_INPUT_SIGNATURE;

    let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}

/// 按相对位移注入鼠标移动事件。
pub unsafe fn simulate_mouse_move_relative(dx: i32, dy: i32) {
    let mut mi = MOUSEINPUT::default();
    mi.dx = dx;
    mi.dy = dy;
    mi.dwFlags = MOUSEEVENTF_MOVE;
    mi.dwExtraInfo = INJECTED_INPUT_SIGNATURE;
    let mut input = INPUT::default();
    input.r#type = INPUT_MOUSE;
    input.Anonymous.mi = mi;
    let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}

/// 按绝对屏幕坐标注入鼠标移动事件。
///
/// `x/y` 为目标位置，`screen_w/screen_h` 用于换算为 Windows 绝对坐标。
pub unsafe fn simulate_mouse_absolute(x: i32, y: i32, screen_w: i32, screen_h: i32) {
    let mut mi = MOUSEINPUT::default();
    mi.dx = (x * 65536) / screen_w;
    mi.dy = (y * 65536) / screen_h;
    mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
    mi.dwExtraInfo = INJECTED_INPUT_SIGNATURE;
    
    let mut input = INPUT::default();
    input.r#type = INPUT_MOUSE;
    input.Anonymous.mi = mi;
    let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}
