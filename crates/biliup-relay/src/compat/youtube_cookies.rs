#![allow(dead_code)]
//! 真实的 Netscape Cookie 格式校验；完整 yt-dlp Cookie 管理待 Phase 2 迁入。

use std::collections::HashSet;
use std::fs;
use std::io;

pub const MAX_COOKIE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct CookieSummary {
    pub count: usize,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Cookie 文件格式无效，请导出 Netscape 格式",
    )
}

pub fn validate_netscape(input: &str) -> io::Result<CookieSummary> {
    if input.len() > MAX_COOKIE_BYTES || input.contains('\0') {
        return Err(invalid());
    }
    if !input.is_empty()
        && !input.lines().next().is_some_and(|line| {
            let line = line.trim_end_matches('\r');
            line == "# Netscape HTTP Cookie File" || line == "# HTTP Cookie File"
        })
    {
        return Err(invalid());
    }
    let mut count = 0;
    let mut keys = HashSet::new();
    for line in input.lines() {
        let line = line.trim_end_matches('\r');
        let line = if let Some(value) = line.strip_prefix("#HttpOnly_") {
            value
        } else {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            line
        };
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() != 7
            || parts[0].is_empty()
            || parts[0]
                .chars()
                .any(|c| c.is_whitespace() || c == '/' || c == ':')
            || !matches!(parts[1], "TRUE" | "FALSE")
            || !parts[2].starts_with('/')
            || !matches!(parts[3], "TRUE" | "FALSE")
            || (!parts[4].is_empty() && parts[4].parse::<u64>().is_err())
            || parts[5..]
                .iter()
                .any(|part| part.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        if !keys.insert((parts[0], parts[2], parts[5])) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cookie 文件包含重复记录",
            ));
        }
        count += 1;
    }
    Ok(CookieSummary { count })
}

/// 清理已退出子进程留下的 yt-dlp Cookie 临时目录。
pub(crate) fn cleanup_stale(directory: &std::path::Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("yt-cookie-")
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let path = entry.path();
        let Some(pid) = fs::read_to_string(path.join("child.pid"))
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        #[cfg(unix)]
        let alive = unsafe { libc::kill(pid as i32, 0) == 0 }
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        #[cfg(windows)]
        let alive = false;
        if !alive {
            let _ = fs::remove_file(path.join("cookies.txt"));
            let _ = fs::remove_file(path.join("child.pid"));
            let _ = fs::remove_dir(path);
        }
    }
}
