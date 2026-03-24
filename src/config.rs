//! 文件作用：管理配置目录、配置列表及当前选中配置的公共状态。

use once_cell::sync::Lazy;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// 存储可用配置的名称与路径。
pub static CONFIGS: Lazy<Mutex<Vec<(String, PathBuf)>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// 记录当前选中的配置索引。
pub static CONFIG_SELECTED: Lazy<Mutex<Option<usize>>> = Lazy::new(|| Mutex::new(None));

/// 解析配置目录路径，优先使用可执行文件同级的 `configs` 目录。
fn get_configs_dir() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            return exe_dir.join("configs");
        }
    }
    PathBuf::from("configs")
}

/// 确保配置目录存在，必要时自动创建。
pub fn ensure_configs_dir() -> bool {
    let configs_dir = get_configs_dir();
    
    if !configs_dir.exists() {
        return fs::create_dir_all(&configs_dir).is_ok();
    }
    true
}

/// 扫描配置目录并刷新全局配置列表。
pub fn load_configs() {
    let mut configs = CONFIGS.lock().unwrap();
    configs.clear();

    let configs_dir = get_configs_dir();
    
    if !configs_dir.exists() {
        return;
    }

    if let Ok(entries) = fs::read_dir(&configs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Some(name) = path.file_stem() {
                    let name_str = name.to_string_lossy().to_string();
                    
                    // 跳过设置文件，避免出现在配置选择列表中。
                    if name_str.to_lowercase() != "settings" {
                        configs.push((name_str, path));
                    }
                }
            }
        }
    }

    configs.sort_by(|a, b| a.0.cmp(&b.0));
}

/// 返回配置目录路径，供其他模块复用。
pub fn get_config_directory() -> PathBuf {
    get_configs_dir()
}
