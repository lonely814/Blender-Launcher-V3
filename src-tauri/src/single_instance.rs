use std::path::{Path, PathBuf};
use tauri::Manager;

/// 单实例守卫：持有排他文件锁直到进程退出。
///
/// 用 `File::try_lock`（Rust 1.89 稳定）而不是写 PID 文件：
/// 文件锁是内核对象，进程崩溃/被杀时自动释放，不会留下死锁。
pub struct InstanceGuard {
    _file: std::fs::File,
    focus_request: PathBuf,
}

impl InstanceGuard {
    /// 锁文件与「求焦点」请求文件都放在 cache 目录；
    /// debug/release 用不同名字，避免 `cargo run` 和已安装版本互相顶掉。
    fn paths(cache_dir: &Path) -> (PathBuf, PathBuf) {
        let name = if cfg!(debug_assertions) { "dev" } else { "app" };
        let lock = cache_dir.join(format!("{name}.lock"));
        let focus = cache_dir.join(format!("{name}.focus"));
        (lock, focus)
    }

    /// 尝试成为唯一实例。Ok(None) 表示已有实例在跑。
    pub fn acquire(cache_dir: &Path) -> Option<Self> {
        let (lock_path, focus_request) = Self::paths(cache_dir);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .ok()?;
        match file.try_lock() {
            Ok(()) => {
                // 清掉上次崩溃可能残留的请求，避免启动即被误唤。
                let _ = std::fs::remove_file(&focus_request);
                Some(Self {
                    _file: file,
                    focus_request,
                })
            }
            Err(_) => None,
        }
    }

    /// 第二个实例调用：给在跑的实例留一个信号，然后自己退出。
    pub fn request_focus(cache_dir: &Path) -> bool {
        let (_, focus) = Self::paths(cache_dir);
        if let Some(parent) = focus.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&focus, "1").is_ok()
    }

    /// 主实例轮询：有人请求焦点就消费掉并返回 true。
    pub fn take_focus_request(&self) -> bool {
        if self.focus_request.exists() {
            let _ = std::fs::remove_file(&self.focus_request);
            true
        } else {
            false
        }
    }
}

/// 单实例 + 唤前台的接线，供 main 调用一次。
///
/// 第二个实例不做 IPC，只写一个标记文件后退出；主实例每 400ms 轮询一次。
/// 这样不需要额外的进程间通信依赖，也不依赖 Win32 的 FindWindow。
pub fn start_watchdog(app: tauri::AppHandle, guard: InstanceGuard) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(400));
        loop {
            ticker.tick().await;
            if guard.take_focus_request() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_is_exclusive_and_released() {
        let dir = std::env::temp_dir().join(format!("bl-inst-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let first = InstanceGuard::acquire(&dir).expect("first acquire");
        // 同一进程内再开一个句柄也必须失败（文件锁是句柄级排他）。
        assert!(
            InstanceGuard::acquire(&dir).is_none(),
            "second instance must be rejected"
        );

        drop(first);
        // 释放后可以重新获得，说明锁随句柄关闭而释放。
        assert!(InstanceGuard::acquire(&dir).is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_focus_request_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bl-focus-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = InstanceGuard::acquire(&dir).unwrap();

        assert!(!guard.take_focus_request());
        assert!(InstanceGuard::request_focus(&dir));
        assert!(guard.take_focus_request());
        // 消费后不重复触发。
        assert!(!guard.take_focus_request());
        std::fs::remove_dir_all(dir).ok();
    }
}
