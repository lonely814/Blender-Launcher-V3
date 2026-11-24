use crate::error::{AppError, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 探测输出前缀。Blender 启动会打印版本 banner、GPU 后端、插件自己的
/// `print`，所以不能整段 stdout 当 JSON 解析——只认带这个前缀的行。
const PROBE_MARKER: &str = "@@BLMGR@@";

/// 硬超时。冷启动几秒，插件多的配置可能到十几秒；20 秒后未返回即杀掉。
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// 探测结果。
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    /// `bpy.context.preferences.addons` 里的全部 module 名。
    /// 混有两类：旧式裸名（`HOps`）与扩展式（`bl_ext.blender_org.CAD_Sketcher`）。
    pub modules: Vec<String>,
    /// 探测失败原因。`modules` 为空且 `error` 为 `Some` 时表示未知，
    /// 不能解读为「没有插件被启用」。
    pub error: Option<String>,
}

/// 从 `bl_ext.<repo>.<pkg_id>` 里取出 `(repo, pkg_id)`。
///
/// 注意 `pkg_id` 本身可能含点（少见但合法），所以只按前两段切分。
pub fn split_extension_module(module: &str) -> Option<(&str, &str)> {
    let rest = module.strip_prefix("bl_ext.")?;
    let (repo, pkg_id) = rest.split_once('.')?;
    (!repo.is_empty() && !pkg_id.is_empty()).then_some((repo, pkg_id))
}

/// 在某个 Build 上探测启用状态。
///
/// 调用方应放到 `spawn_blocking`：这是一个可能跑 20 秒的子进程。
pub fn probe_enabled(blender_exe: &Path) -> ProbeResult {
    if !blender_exe.is_file() {
        return ProbeResult {
            modules: Vec::new(),
            error: Some(format!("找不到可执行文件: {}", blender_exe.display())),
        };
    }

    let expr = format!(
        "import bpy,json;print('{PROBE_MARKER}'+json.dumps(sorted(bpy.context.preferences.addons.keys())))"
    );

    let mut child = match std::process::Command::new(blender_exe)
        .args(["-b", "--python-expr", &expr])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ProbeResult {
                modules: Vec::new(),
                error: Some(format!("启动 Blender 失败: {err}")),
            }
        }
    };

    // 没有 wait_timeout crate，用轮询实现超时等待。
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= PROBE_TIMEOUT {
                    // 超时即杀。留着会让后续探测全部排队。
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                let _ = child.kill();
                return ProbeResult {
                    modules: Vec::new(),
                    error: Some(format!("等待 Blender 退出失败: {err}")),
                };
            }
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }

    if status.is_none() {
        return ProbeResult {
            modules: Vec::new(),
            error: Some(format!("探测超时（{} 秒）", PROBE_TIMEOUT.as_secs())),
        };
    }

    match parse_probe_stdout(&stdout) {
        Some(modules) => ProbeResult {
            modules,
            error: None,
        },
        None => {
            let excerpt: String = stderr.chars().take(500).collect();
            ProbeResult {
                modules: Vec::new(),
                error: Some(format!("输出中找不到探测标记。stderr: {excerpt}")),
            }
        }
    }
}

/// 从 stdout 里找到带标记的那一行并解析 JSON 数组。
pub fn parse_probe_stdout(stdout: &str) -> Option<Vec<String>> {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix(PROBE_MARKER))
        .and_then(|json| serde_json::from_str(json).ok())
}

/// 判断某个仓库下的某个包是否启用。
///
/// 必须按完整的 `bl_ext.<repo>.<pkg_id>` 比对，不能只比 pkg_id——
/// 同名扩展可能同时存在于 `blender_org` 和 `blender4_com`。
pub fn is_extension_enabled(modules: &[String], repo: &str, pkg_id: &str) -> bool {
    let target = format!("bl_ext.{repo}.{pkg_id}");
    modules.iter().any(|module| module == &target)
}

/// 判断旧式 addon 是否启用。裸模块名直接比对。
pub fn legacy_enabled(modules: &[String], module: &str) -> bool {
    modules
        .iter()
        .any(|item| item == module || item.starts_with(&format!("{module}.")))
}

/// 定位 Build 的 Blender 可执行文件。
pub fn blender_executable(install_path: &Path) -> Option<PathBuf> {
    const CANDIDATES: [&str; 3] = ["blender.exe", "Blender", "blender"];

    // 便携构建把 exe 放在 `portable/..`、也可能直接在根目录。
    let mut dirs = vec![install_path.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(install_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }

    for dir in dirs {
        for name in CANDIDATES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 把探测结果写进扫描结果（在 `service::extensions::scan` 之后调用）。
pub fn attach_enabled(
    scan: &mut crate::models::extension::ExtensionScan,
    result: &ProbeResult,
) -> Result<()> {
    if result.error.is_some() {
        // 探测失败时保持 `None`，UI 显示未知。绝不写成 `false`——
        // 「未知」和「已禁用」对用户的行动含义完全不同。
        return Ok(());
    }

    for extension in &mut scan.extensions {
        // 只认扩展式模块名；旧式目录不该出现在扩展仓库下。
        extension.enabled = Some(is_extension_enabled(
            &result.modules,
            &extension.repo,
            &extension.pkg_id,
        ));
    }

    for addon in &mut scan.legacy {
        addon.enabled = Some(legacy_enabled(&result.modules, &addon.module));
    }

    Ok(())
}

/// 校验可执行文件路径，避免把任意路径传给 shell。
pub fn validate_exe(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(AppError::InvalidSettings(format!(
            "不是文件: {}",
            path.display()
        )));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::InvalidSettings("可执行文件名非法".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_extension_module() {
        assert_eq!(
            split_extension_module("bl_ext.blender_org.CAD_Sketcher"),
            Some(("blender_org", "CAD_Sketcher"))
        );
        assert_eq!(
            split_extension_module("bl_ext.blender4_com.b4d_tools"),
            Some(("blender4_com", "b4d_tools"))
        );
        // pkg_id 含点时只按前两段切，剩下都算 pkg_id。
        assert_eq!(
            split_extension_module("bl_ext.blender_org.some.pkg"),
            Some(("blender_org", "some.pkg"))
        );
        // 旧式 addon 没有前缀。
        assert_eq!(split_extension_module("HOps"), None);
        assert_eq!(split_extension_module("bl_ext."), None);
        assert_eq!(split_extension_module("bl_ext.repo"), None);
    }

    #[test]
    fn test_parse_probe_stdout_picks_marked_line() {
        // 真实输出混杂大量插件日志，中间那行才是结果。
        let stdout = r#"
[Light Wrangler] [INFO] starting...
Blender 5.3.0 (hash 02f0899d78e3 built 2026-11-10)
@@BLMGR@@["HOps", "bl_ext.blender_org.CAD_Sketcher"]
Warning keymap not found
"#;
        let modules = parse_probe_stdout(stdout).expect("应解析出模块列表");
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[1], "bl_ext.blender_org.CAD_Sketcher");
    }

    #[test]
    fn test_parse_probe_stdout_missing_marker_is_none() {
        assert_eq!(parse_probe_stdout("Blender quit\n"), None);
        // 有标记但 JSON 坏了也要 None，不能猜。
        assert_eq!(parse_probe_stdout("@@BLMGR@@not json"), None);
    }

    #[test]
    fn test_parse_probe_stdout_empty_list() {
        let modules = parse_probe_stdout("@@BLMGR@@[]").expect("空列表是合法结果");
        assert!(modules.is_empty());
    }

    #[test]
    fn test_legacy_enabled_matches_bare_and_submodule() {
        let modules = vec![
            "HOps".to_string(),
            "some_addon.submodule".to_string(),
            "bl_ext.blender_org.facetor".to_string(),
        ];
        assert!(legacy_enabled(&modules, "HOps"));
        assert!(legacy_enabled(&modules, "some_addon"));
        // 扩展式模块名不应被当成旧式 addon 命中。
        assert!(!legacy_enabled(&modules, "blender_org"));
        assert!(!legacy_enabled(&modules, "facetor"));
    }

    #[test]
    fn test_is_extension_enabled_distinguishes_repos() {
        let modules = vec!["bl_ext.blender_org.facetor".to_string()];

        assert!(is_extension_enabled(&modules, "blender_org", "facetor"));
        // 同名扩展在不同仓库下不应互相命中。
        assert!(!is_extension_enabled(&modules, "blender4_com", "facetor"));
        assert!(!is_extension_enabled(&modules, "blender_org", "other"));
    }

    #[test]
    fn test_attach_enabled_leaves_none_on_probe_failure() {
        use crate::models::extension::{ExtensionScan, InstalledExtension};

        let mut scan = ExtensionScan {
            extensions: vec![InstalledExtension {
                pkg_id: "facetor".to_string(),
                repo: "blender_org".to_string(),
                path: PathBuf::from("x"),
                manifest: Default::default(),
                enabled: None,
                managed: None,
                size_bytes: 0,
            }],
            ..ExtensionScan::default()
        };

        let failed = ProbeResult {
            modules: Vec::new(),
            error: Some("timeout".to_string()),
        };
        attach_enabled(&mut scan, &failed).unwrap();
        assert_eq!(
            scan.extensions[0].enabled, None,
            "探测失败必须保持未知，不能写成 false"
        );

        let ok = ProbeResult {
            modules: vec!["bl_ext.blender_org.facetor".to_string()],
            error: None,
        };
        attach_enabled(&mut scan, &ok).unwrap();
        assert_eq!(scan.extensions[0].enabled, Some(true));
    }

    #[test]
    fn test_validate_exe_rejects_traversal() {
        assert!(validate_exe(Path::new("C:/definitely/not/here.exe")).is_err());
        // 存在的文件才可能通过；名字含路径分隔符的一律拒。
        let bogus = Path::new("..\\cmd.exe");
        assert!(validate_exe(bogus).is_err());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// 真机验证：在 `D:\Blender\stable\blender-5.3.0-windows-x64` 上探测。
    ///
    /// 手动运行：`cargo test --lib live_probe -- --ignored --nocapture`
    #[test]
    #[ignore = "需要本机装有 Blender"]
    fn live_probe_real_blender() {
        let exe = Path::new("D:/Blender/stable/blender-5.3.0-windows-x64/blender.exe");
        if !exe.is_file() {
            return;
        }
        let result = probe_enabled(exe);
        println!("error = {:?}", result.error);
        println!("module count = {}", result.modules.len());
        assert!(result.error.is_none(), "探测应成功: {:?}", result.error);
        assert!(!result.modules.is_empty(), "应有已启用插件");

        // 本机实测已知存在这两项。
        assert!(
            is_extension_enabled(&result.modules, "blender_org", "CAD_Sketcher"),
            "CAD_Sketcher 应识别为启用"
        );
        assert!(legacy_enabled(&result.modules, "HOps"), "HOps 应识别为启用");
        // 一个确定不存在的包不应命中。
        assert!(!is_extension_enabled(
            &result.modules,
            "blender_org",
            "definitely_not_installed_pkg"
        ));
    }
}
