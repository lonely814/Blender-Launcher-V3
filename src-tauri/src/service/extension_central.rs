//! 阶段 B：中央库 + junction 版本切换。
//!
//! 布局：
//! ```text
//! <central>/<pkg_id>/<version>/          ← 真实内容
//! <ext_root>/<repo>/<pkg_id>  ──junction──>  <central>/<pkg_id>/<version>/
//! ```
//!
//! 与本模块外的 `extensions::scan` 严格分工：扫描只读，本模块只改文件系统。
//!
//! ## 为什么必须是 junction 而不是复制
//!
//! 同一扩展的多个版本要能切换而不用重新下载。junction 让 `<pkg_id>` 目录
//! 始终指向中央库里当前选中的那版，Blender 看到的仍是普通目录。
//!
//! ## 为什么删除只用 `remove_dir` / `remove_file`
//!
//! 2026-11-24 实测（rustc 1.99.1 / Windows 11 26200）：
//! - `std::fs::read_dir` 会**穿透** junction，读到中央库里的真实内容；
//! - `std::fs::remove_dir_all` 对 junction 只删链接本身，中央库内容完好；
//! - `std::fs::remove_dir` 对 junction 同样只删链接。
//!
//! 但那两条「安全」行为是实现细节，不是接口承诺。Blender 自己的卸载走
//! Python `shutil.rmtree`，它对 junction 抛 `OSError: Cannot call rmtree on a
//! symbolic link`，再由回调改用 `os.unlink`——也是安全的，但同样依赖实现细节。
//! 所以本模块一律先 `symlink_metadata` 判类型，junction 走 `remove_dir`，
//! 绝不对可能穿透的目录调用递归删除。

use crate::error::{AppError, Result};
use crate::models::extension::{ManagedMetadata, MANAGED_MARKER, MANAGED_SCHEMA_VERSION};
use crate::service::extensions;
use std::path::{Path, PathBuf};

/// 判断路径是否是指向目录的链接（junction 或符号链接）。
pub fn is_dir_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// 建目录链接。Windows 上符号链接需开发者模式/管理员，失败退 junction。
///
/// 与 `commands/build_entry.rs` 的 `create_symlink` 同一条路：
/// junction 不需要提权，是本程序能默认启用的唯一选项。
pub fn create_dir_link(target: &Path, link: &Path) -> Result<()> {
    if link.exists() || is_dir_link(link) {
        return Err(AppError::InvalidSettings(format!(
            "链接目标已存在：{}",
            link.display()
        )));
    }

    #[cfg(target_os = "windows")]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_err() {
            let output = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.display().to_string(),
                    &target.display().to_string(),
                ])
                .output()
                .map_err(AppError::FileSystem)?;
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Err(AppError::FileSystem(std::io::Error::other(format!(
                    "创建目录链接失败：{detail}"
                ))));
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    std::os::unix::fs::symlink(target, link).map_err(AppError::FileSystem)?;

    Ok(())
}

/// 删除链接本身，不动目标内容。
///
/// 先判类型再动手：对 junction 调递归删除在多数 Rust/Python 实现里是安全的，
/// 但那是实现细节；显式判断让行为不随标准库版本变化。
pub fn remove_link(path: &Path) -> Result<()> {
    if !is_dir_link(path) {
        return Err(AppError::InvalidSettings(format!(
            "{} 不是链接，拒绝按链接删除",
            path.display()
        )));
    }
    std::fs::remove_dir(path).map_err(AppError::FileSystem)
}

/// 递归复制目录。符号链接按内容复制，不复制链接本身。
fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(AppError::FileSystem)?;
    for entry in std::fs::read_dir(src)
        .map_err(AppError::FileSystem)?
        .flatten()
    {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let file_type = entry.file_type().map_err(AppError::FileSystem)?;
        if file_type.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(AppError::FileSystem)?;
        }
    }
    Ok(())
}

/// 对比两棵目录树：相对路径集合 + 每个文件的大小。
///
/// 不比内容哈希：接管前的复制是本机磁盘操作，搬错的概率远低于
/// 「因为某个文件被占用而没复制成功」，比对规模能抓住这一点就够了。
/// 代价是多读一遍目录元数据，不读文件内容。
fn same_tree(left: &Path, right: &Path) -> bool {
    fn collect(root: &Path, base: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                collect(&path, base, out);
            } else if let Ok(relative) = path.strip_prefix(base) {
                let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                out.push((relative.to_path_buf(), size));
            }
        }
    }

    let mut left_entries = Vec::new();
    let mut right_entries = Vec::new();
    collect(left, left, &mut left_entries);
    collect(right, right, &mut right_entries);

    left_entries.sort_unstable();
    right_entries.sort_unstable();
    left_entries == right_entries
}

/// 中央库里某扩展已有的版本目录名（按版本降序）。
pub fn versions(central_root: &Path, pkg_id: &str) -> Vec<String> {
    let pkg_dir = central_root.join(pkg_id);
    let Ok(entries) = std::fs::read_dir(&pkg_dir) else {
        return Vec::new();
    };
    let mut versions: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    // 降序：UI 默认显示最新一版。非语义化版本名排在后面由排序自然处理。
    versions.sort_by(|left, right| right.cmp(left));
    versions
}

/// 接管一个已装扩展：复制进中央库、校验、换链接，全程可回滚。
///
/// 步骤顺序是有意为之：
/// 1. 复制到中央库（原目录还在，随时可放弃）；
/// 2. 校验两棵树一致（不一致就删掉副本，原目录未动）；
/// 3. 把原目录**改名**为备份（同卷 rename，原子且瞬时）；
/// 4. 在原名建链接（失败就把备份改回来）；
/// 5. 删备份、写标记。
///
/// 原目录在最后一步之前始终是完整可用的，中途断电最坏情况是留下一个
/// `<pkg>.blmgr-bak` 目录，用户手工改回名字即可。
pub fn adopt(
    pkg_path: &Path,
    central_root: &Path,
    pkg_id: &str,
    version: &str,
) -> Result<ManagedMetadata> {
    if !pkg_path.is_dir() {
        return Err(AppError::InvalidSettings(format!(
            "{} 不是目录",
            pkg_path.display()
        )));
    }
    if is_dir_link(pkg_path) {
        return Err(AppError::InvalidSettings(format!(
            "{} 已是链接，无需接管",
            pkg_path.display()
        )));
    }
    let version = sanitize_component(version)?;
    let pkg_id = sanitize_component(pkg_id)?;

    let central_path = central_root.join(&pkg_id).join(&version);
    if central_path.exists() {
        return Err(AppError::InvalidSettings(format!(
            "中央库已有同名版本：{}",
            central_path.display()
        )));
    }

    copy_dir(pkg_path, &central_path)?;
    if !same_tree(pkg_path, &central_path) {
        let _ = std::fs::remove_dir_all(&central_path);
        return Err(AppError::FileSystem(std::io::Error::other(
            "复制到中央库后校验不一致，已放弃接管",
        )));
    }

    let backup = pkg_path.with_extension("blmgr-bak");
    if backup.exists() {
        let _ = std::fs::remove_dir_all(&central_path);
        return Err(AppError::InvalidSettings(format!(
            "备份路径已存在，请先清理：{}",
            backup.display()
        )));
    }
    std::fs::rename(pkg_path, &backup).map_err(AppError::FileSystem)?;

    if let Err(err) = create_dir_link(&central_path, pkg_path) {
        // 建链接失败：把原目录改回名字，删掉刚复制的副本。
        let restored = std::fs::rename(&backup, pkg_path);
        let _ = std::fs::remove_dir_all(&central_path);
        return match restored {
            Ok(()) => Err(err),
            Err(restore_err) => Err(AppError::FileSystem(std::io::Error::other(format!(
                "建链接失败（{err}），且还原原目录也失败（{restore_err}）。原内容在 {}",
                backup.display()
            )))),
        };
    }

    std::fs::remove_dir_all(&backup).map_err(AppError::FileSystem)?;

    let marker = ManagedMetadata {
        schema_version: MANAGED_SCHEMA_VERSION,
        central_path: central_path.clone(),
        pkg_id: pkg_id.clone(),
        managed_since: chrono::Utc::now(),
    };
    // 通过链接写入，标记落在中央库里，跟着版本走。
    extensions::write_managed_marker(pkg_path, &marker)?;
    Ok(marker)
}

/// 切换已被接管的扩展到中央库里的另一个版本。
pub fn switch_version(
    pkg_path: &Path,
    central_root: &Path,
    pkg_id: &str,
    version: &str,
) -> Result<ManagedMetadata> {
    let marker = read_marker_or_error(pkg_path)?;
    let pkg_id = sanitize_component(pkg_id)?;
    let version = sanitize_component(version)?;
    let target = central_root.join(&pkg_id).join(&version);
    if !target.is_dir() {
        return Err(AppError::InvalidSettings(format!(
            "中央库没有该版本：{}",
            target.display()
        )));
    }
    if !is_dir_link(pkg_path) {
        return Err(AppError::InvalidSettings(format!(
            "{} 不是链接，拒绝切换",
            pkg_path.display()
        )));
    }

    // 先删链接再建新链接。失败时把旧链接建回来，保持扩展可用。
    let previous = marker.central_path.clone();
    remove_link(pkg_path)?;
    if let Err(err) = create_dir_link(&target, pkg_path) {
        let _ = create_dir_link(&previous, pkg_path);
        return Err(err);
    }

    let updated = ManagedMetadata {
        schema_version: MANAGED_SCHEMA_VERSION,
        central_path: target,
        pkg_id,
        managed_since: marker.managed_since,
    };
    extensions::write_managed_marker(pkg_path, &updated)?;
    Ok(updated)
}

/// 解除接管：把内容复制回原位置，删链接与标记。中央库保留（可再次接管）。
pub fn release(pkg_path: &Path) -> Result<()> {
    let marker = read_marker_or_error(pkg_path)?;
    if !is_dir_link(pkg_path) {
        return Err(AppError::InvalidSettings(format!(
            "{} 不是链接，无需解除",
            pkg_path.display()
        )));
    }
    if !marker.central_path.is_dir() {
        return Err(AppError::InvalidSettings(format!(
            "中央库内容已不存在：{}",
            marker.central_path.display()
        )));
    }

    let restore = pkg_path.with_extension("blmgr-restore");
    if restore.exists() {
        return Err(AppError::InvalidSettings(format!(
            "还原路径已存在，请先清理：{}",
            restore.display()
        )));
    }

    copy_dir(&marker.central_path, &restore)?;
    if !same_tree(&marker.central_path, &restore) {
        let _ = std::fs::remove_dir_all(&restore);
        return Err(AppError::FileSystem(std::io::Error::other(
            "复制回原位置后校验不一致，已放弃解除接管",
        )));
    }
    // 标记文件不属于扩展内容，别让它留在还原后的目录里。
    let _ = std::fs::remove_file(restore.join(MANAGED_MARKER));

    remove_link(pkg_path)?;
    if let Err(err) = std::fs::rename(&restore, pkg_path) {
        // 链接已删但改名失败：把链接建回来，至少扩展仍可用。
        let _ = create_dir_link(&marker.central_path, pkg_path);
        let _ = std::fs::remove_dir_all(&restore);
        return Err(err.into());
    }
    Ok(())
}

/// 读标记并要求它属于本程序。没有标记 = 不是本程序接管 = 一律拒绝改写。
fn read_marker_or_error(pkg_path: &Path) -> Result<ManagedMetadata> {
    extensions::read_managed_marker(pkg_path).ok_or_else(|| {
        AppError::InvalidSettings(format!(
            "{} 没有有效的 {}，只读",
            pkg_path.display(),
            MANAGED_MARKER
        ))
    })
}

/// 版本号与 pkg_id 会成为目录名，必须挡掉路径分隔符与相对路径片段。
fn sanitize_component(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
    {
        return Err(AppError::InvalidSettings(format!(
            "非法的目录名成分：{value:?}"
        )));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 唯一临时目录：不为几个测试引入 tempfile 依赖。
    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "blmgr-central-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_pkg(root: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let pkg = root.join(name);
        std::fs::create_dir_all(&pkg).unwrap();
        for (relative, content) in files {
            let path = pkg.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        pkg
    }

    #[test]
    fn test_sanitize_component_rejects_traversal() {
        assert!(sanitize_component("1.0.1").is_ok());
        assert!(sanitize_component("").is_err());
        assert!(sanitize_component("..").is_err());
        assert!(sanitize_component("a/b").is_err());
        assert!(sanitize_component("a\\b").is_err());
        assert!(sanitize_component("C:evil").is_err());
    }

    #[test]
    fn test_same_tree_detects_missing_and_changed_files() {
        let root = unique_temp_dir();
        let left = make_pkg(&root, "left", &[("a.txt", "1"), ("sub/b.txt", "22")]);
        let right = make_pkg(&root, "right", &[("a.txt", "1"), ("sub/b.txt", "22")]);
        assert!(same_tree(&left, &right));

        std::fs::write(right.join("sub/b.txt"), "different-length").unwrap();
        assert!(!same_tree(&left, &right));

        std::fs::remove_file(right.join("a.txt")).unwrap();
        assert!(!same_tree(&left, &right));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_adopt_moves_content_and_writes_marker() {
        let root = unique_temp_dir();
        let central = root.join("central");
        let pkg = make_pkg(
            &root,
            "mypkg",
            &[("__init__.py", "print(1)"), ("sub/x.txt", "abc")],
        );

        let marker = adopt(&pkg, &central, "mypkg", "1.0.1").unwrap();
        assert_eq!(marker.schema_version, MANAGED_SCHEMA_VERSION);
        assert!(is_dir_link(&pkg), "接管后原位置应变成链接");
        assert!(central
            .join("mypkg")
            .join("1.0.1")
            .join("__init__.py")
            .is_file());
        // 通过链接仍能读到内容。
        assert!(pkg.join("sub/x.txt").is_file());
        // 标记可被扫描层识别。
        let read_back = extensions::read_managed_marker(&pkg).unwrap();
        assert_eq!(read_back.pkg_id, "mypkg");

        // 解除接管后应恢复成普通目录，且内容还在。
        release(&pkg).unwrap();
        assert!(!is_dir_link(&pkg));
        assert!(pkg.join("__init__.py").is_file());
        assert!(pkg.join("sub/x.txt").is_file());
        assert!(!pkg.join(MANAGED_MARKER).exists());
        // 中央库保留，供再次接管。
        assert!(central
            .join("mypkg")
            .join("1.0.1")
            .join("__init__.py")
            .is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_adopt_refuses_existing_central_version() {
        let root = unique_temp_dir();
        let central = root.join("central");
        let pkg = make_pkg(&root, "dup", &[("a.txt", "1")]);
        std::fs::create_dir_all(central.join("dup").join("1.0.1")).unwrap();

        let err = adopt(&pkg, &central, "dup", "1.0.1").unwrap_err();
        assert!(err.to_string().contains("中央库已有同名版本"));
        assert!(!is_dir_link(&pkg), "失败时不能动原目录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_switch_version_repoints_link() {
        let root = unique_temp_dir();
        let central = root.join("central");
        let pkg = make_pkg(
            &root,
            "sw",
            &[("blender_manifest.toml", "version = \"1.0.1\"")],
        );
        adopt(&pkg, &central, "sw", "1.0.1").unwrap();

        // 手工构造第二版。
        let second = central.join("sw").join("2.0.0");
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("blender_manifest.toml"), "version = \"2.0.0\"").unwrap();

        let marker = switch_version(&pkg, &central, "sw", "2.0.0").unwrap();
        assert_eq!(marker.central_path, second);
        let content = std::fs::read_to_string(pkg.join("blender_manifest.toml")).unwrap();
        assert!(content.contains("2.0.0"));

        // 切到不存在的版本必须失败，且链接仍指向原来的版本。
        assert!(switch_version(&pkg, &central, "sw", "9.9.9").is_err());
        let content = std::fs::read_to_string(pkg.join("blender_manifest.toml")).unwrap();
        assert!(content.contains("2.0.0"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_unmarked_directory_is_rejected() {
        let root = unique_temp_dir();
        let central = root.join("central");
        // 用户自己装的插件：普通目录、无标记。
        let pkg = make_pkg(&root, "userpkg", &[("a.txt", "1")]);

        assert!(switch_version(&pkg, &central, "userpkg", "1.0.0").is_err());
        assert!(release(&pkg).is_err());
        assert!(!is_dir_link(&pkg));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_versions_sorted_descending() {
        let root = unique_temp_dir();
        std::fs::create_dir_all(root.join("p").join("1.0.0")).unwrap();
        std::fs::create_dir_all(root.join("p").join("2.1.0")).unwrap();
        std::fs::create_dir_all(root.join("p").join("2.0.9")).unwrap();
        std::fs::write(root.join("p").join("not-a-dir.txt"), "x").unwrap();

        let list = versions(&root, "p");
        assert_eq!(list, vec!["2.1.0", "2.0.9", "1.0.0"]);
        assert!(versions(&root, "missing").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_remove_link_refuses_plain_directory() {
        let root = unique_temp_dir();
        let plain = make_pkg(&root, "plain", &[("a.txt", "1")]);
        assert!(remove_link(&plain).is_err());
        // 普通目录与内容必须原样保留。
        assert!(plain.join("a.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 真机验证：在真实扩展根上完成「接管 → 切换 → 解除」全流程。
///
/// 用一次性包 `blmgr_adopt_probe`，不碰用户已有插件。跑完必须自己清理，
/// 包括中央库里的副本与残留链接。
#[cfg(test)]
mod live_tests {
    use super::*;

    const EXT_ROOT: &str = "C:/Blender/stable/blender-5.2.0-windows-x64/portable/extensions";
    const CENTRAL: &str = "C:/Temp/blmgr_central_live";
    const PKG: &str = "blmgr_adopt_probe";

    #[test]
    #[ignore]
    fn live_adopt_switch_release_on_real_ext_root() {
        let ext_root = Path::new(EXT_ROOT);
        let pkg_path = ext_root.join("user_default").join(PKG);
        if !pkg_path.is_dir() {
            eprintln!("skip: {PKG} not present at {}", pkg_path.display());
            return;
        }

        let central = Path::new(CENTRAL);
        let _ = std::fs::remove_dir_all(central);
        std::fs::create_dir_all(central).unwrap();

        // 1. 接管
        let marker = adopt(&pkg_path, central, PKG, "0.0.1").expect("adopt failed");
        assert!(is_dir_link(&pkg_path), "接管后应是链接");
        assert!(central.join(PKG).join("0.0.1").is_dir(), "中央库应有内容");
        assert!(
            pkg_path.join(MANAGED_MARKER).is_file(),
            "标记应可见（通过链接写入中央库）"
        );
        assert_eq!(marker.pkg_id, PKG);

        // 2. 准备第二版并切换
        std::fs::create_dir_all(central.join(PKG).join("0.0.2")).unwrap();
        std::fs::write(central.join(PKG).join("0.0.2").join("marker.txt"), "v2").unwrap();
        let switched = switch_version(&pkg_path, central, PKG, "0.0.2").expect("switch failed");
        assert!(switched.central_path.ends_with("0.0.2"));
        assert!(
            pkg_path.join("marker.txt").is_file(),
            "切换后链接应指向新版本"
        );

        // 3. 解除接管
        release(&pkg_path).expect("release failed");
        assert!(!is_dir_link(&pkg_path), "解除后应是普通目录");
        assert!(pkg_path.join("marker.txt").is_file(), "解除后内容要回来");
        assert!(!pkg_path.join(MANAGED_MARKER).exists(), "标记应被清除");
        assert!(central.join(PKG).join("0.0.2").is_dir(), "中央库保留");

        // 清理
        let _ = std::fs::remove_dir_all(&pkg_path);
        let _ = std::fs::remove_dir_all(central);
        eprintln!("live adopt/switch/release ok");
    }
}
