//! 子进程构造：统一在 Windows 上抑制控制台窗口。
//!
//! FFmpeg / FFprobe 是控制台程序。如果父进程（Tauri 桌面应用）直接 `Command::new`
//! 启动它们，Windows 会为每个子进程新建并显示一个控制台窗口 —— 表现就是
//! 「开始转换时连续弹出几个透明窗口」（一次任务 decode + encode 会 spawn 多次）。
//! 设置 `CREATE_NO_WINDOW` 可以彻底避免，且不影响 stdout/stderr 管道读取。

use std::ffi::OsStr;
use std::process::Command;

/// Windows 创建标志：不为新进程创建控制台窗口。
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 构造子进程命令（本 crate 内所有 spawn 都必须走这里）。
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_no_window_flag_matches_winbase() {
        #[cfg(windows)]
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    #[test]
    fn helper_still_spawns_and_captures_output() {
        // 证明加了标志后仍能正常启动并读到输出
        #[cfg(windows)]
        let output = command("cmd")
            .args(["/C", "echo", "tf-ok"])
            .output()
            .expect("cmd 应可执行");
        #[cfg(not(windows))]
        let output = command("echo").arg("tf-ok").output().expect("echo 应可执行");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("tf-ok"));
    }
}
