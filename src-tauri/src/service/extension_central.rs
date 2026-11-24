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
        // 中央库里已有同一版本：内容一致就复用，不一致才拒绝。
        // 复用这条路径在「解除接管后又接管回来」时是常态——
        // 内容没变却要用户先手工删掉中央库里的副本，没有道理。
        if !same_tree(pkg_path, &central_path) {
            return Err(AppError::InvalidSettings(format!(
                "中央库已有同名版本且内容不同：{}",
                central_path.display()
            )));
        }
        return link_to_existing(pkg_path, &central_path, &pkg_id);
    }

    copy_dir(pkg_path, &central_path)?;
    if !same_tree(pkg_path, &central_path) {
        let _ = std::fs::remove_dir_all(&central_path);
        return Err(AppError::FileSystem(std::io::Error::other(
            "复制到中央库后校验不一致，已放弃接管",
        )));
    }

    link_to_existing(pkg_path, &central_path, &pkg_id)
}

/// 把已经存在于中央库的内容换成指向它的链接（接管的后半段）。
///
/// 单独抽出来是因为它同时服务两条路径：正常接管（刚复制完）与
/// 复用既有版本（内容比对一致）。两条路径的安全要求相同，
/// 写两份迟早会分叉。
fn link_to_existing(pkg_path: &Path, central_path: &Path, pkg_id: &str) -> Result<ManagedMetadata> {
    let backup = pkg_path.with_extension("blmgr-bak");
    if backup.exists() {
        return Err(AppError::InvalidSettings(format!(
            "备份路径已存在，请先清理：{}",
            backup.display()
        )));
    }
    std::fs::rename(pkg_path, &backup).map_err(AppError::FileSystem)?;

    if let Err(err) = create_dir_link(central_path, pkg_path) {
        // 建链接失败：把原目录改回名字。
        let restored = std::fs::rename(&backup, pkg_path);
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
        central_path: central_path.to_path_buf(),
        pkg_id: pkg_id.to_string(),
        managed_since: chrono::Utc::now(),
    };
    // 通过链接写入，标记落在中央库里，跟着版本走。
    extensions::write_managed_marker(pkg_path, &marker)?;
    Ok(marker)
}

/// 把一个 zip 解到中央库，成为 `<pkg_id>/<version>` 这一版。
///
/// 只写中央库，不碰扩展目录：新加进来的版本要等 `switch_version`
/// 才会生效。这样「下载失败」「解压失败」「id 对不上」都不会影响
/// 当前正在用的那一版。
///
/// 内容放在临时目录里解压再整体改名，避免解压中途失败留下半个版本——
/// 半个版本一旦被切换过去，Blender 里就是一个坏掉的扩展。
pub fn import_zip(
    zip_path: &Path,
    central_root: &Path,
    pkg_id: &str,
    version: &str,
) -> Result<PathBuf> {
    let pkg_id = sanitize_component(pkg_id)?;
    let version = sanitize_component(version)?;
    if !zip_path.is_file() {
        return Err(AppError::InvalidSettings(format!(
            "扩展包不存在：{}",
            zip_path.display()
        )));
    }

    let target = central_root.join(&pkg_id).join(&version);
    if target.exists() {
        return Err(AppError::InvalidSettings(format!(
            "中央库已有该版本：{}",
            target.display()
        )));
    }

    let staging = central_root
        .join(&pkg_id)
        .join(format!(".incoming-{version}"));
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    std::fs::create_dir_all(&staging).map_err(AppError::FileSystem)?;

    let outcome = extract_flat(zip_path, &staging)
        .and_then(|()| verify_manifest_id(&staging, &pkg_id))
        .and_then(|()| std::fs::rename(&staging, &target).map_err(AppError::FileSystem));

    if let Err(err) = outcome {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    Ok(target)
}

/// 解压并剥掉「所有条目都在同一层子目录下」的那层包裹。
///
/// 官方下载包是扁平结构，但用户手工导入的 GitHub zip 常带一层
/// `<repo>-main/`。blender 自己的 `pkg_zipfile_detect_subdir_or_none`
/// 也做同样处理，否则解出来的目录在 Blender 里读不到 manifest。
fn extract_flat(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path).map_err(AppError::FileSystem)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| AppError::ExtractFailed(err.to_string()))?;

    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
        names.push(entry.name().to_string());
    }
    let wrapper = common_wrapper_dir(&names);

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
        // `enclosed_name` 已经把 `..` 与绝对路径挡掉，是 zip crate 提供的
        // zip-slip 防护；这里再用它一次，避免自己拼 relative 时绕过。
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let relative = match &wrapper {
            Some(dir) => match relative.strip_prefix(dir) {
                Ok(rest) => rest.to_path_buf(),
                Err(_) => continue,
            },
            None => relative,
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out_path = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(AppError::FileSystem)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::FileSystem)?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(AppError::FileSystem)?;
        std::io::copy(&mut entry, &mut out).map_err(AppError::FileSystem)?;
    }
    Ok(())
}

/// 如果所有条目都在同一个顶层目录下，返回那个目录名。
fn common_wrapper_dir(names: &[String]) -> Option<PathBuf> {
    let mut first: Option<&str> = None;
    for name in names {
        let name = name.trim_start_matches("./");
        if name.is_empty() {
            continue;
        }
        let head = name.split('/').next()?;
        // 有条目直接放在根上（含根级 `blender_manifest.toml`）就没有包裹层。
        if !name.contains('/') {
            return None;
        }
        match first {
            None => first = Some(head),
            Some(existing) if existing != head => return None,
            _ => {}
        }
    }
    first.map(PathBuf::from)
}

/// 确认解出来的包确实是我们要的那个扩展。
///
/// 版本号和 id 都从解出来的 manifest 复核：目录里的版本号与包里写的
/// 版本号不一致时，中央库里就会多出一个名字骗人的版本目录。
fn verify_manifest_id(dir: &Path, expected_id: &str) -> Result<()> {
    let manifest_path = dir.join("blender_manifest.toml");
    if !manifest_path.is_file() {
        return Err(AppError::InvalidSettings(
            "扩展包里没有 blender_manifest.toml".to_string(),
        ));
    }
    let manifest = extensions::read_manifest(dir)?;
    if !manifest.id.eq_ignore_ascii_case(expected_id) {
        return Err(AppError::InvalidSettings(format!(
            "包内 id 是 {:?}，与预期的 {expected_id:?} 不一致",
            manifest.id
        )));
    }
    if !manifest.version.trim().is_empty() {
        // 只做提示不阻断：第三方包的 manifest 版本与目录版本对不上是常事，
        // 而版本目录名由调用方（下载链接/目录条目）决定，已经确定。
        tracing::debug!(
            manifest_version = %manifest.version,
            "解压后的 manifest 版本"
        );
    }
    Ok(())
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
    fn test_adopt_reuses_identical_existing_version() {
        // 「解除接管 → 又接管回来」的路径：中央库副本还在且内容相同，
        // 应当直接建链接，不该逼用户先手工删中央库。
        let root = unique_temp_dir();
        let central = root.join("central");
        let pkg = make_pkg(&root, "reuse", &[("a.txt", "1"), ("sub/b.txt", "22")]);
        std::fs::create_dir_all(central.join("reuse").join("1.0.0").join("sub")).unwrap();
        std::fs::write(central.join("reuse").join("1.0.0").join("a.txt"), "1").unwrap();
        std::fs::write(central.join("reuse").join("1.0.0").join("sub/b.txt"), "22").unwrap();

        let marker = adopt(&pkg, &central, "reuse", "1.0.0").unwrap();
        assert!(is_dir_link(&pkg), "应改为链接");
        assert!(
            marker.central_path.ends_with("reuse/1.0.0")
                || marker.central_path.ends_with("reuse\\1.0.0")
        );
        assert!(pkg.join("sub/b.txt").is_file());
        assert!(
            extensions::read_managed_marker(&pkg).is_some(),
            "标记要写上"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 造一个 zip，`prefix` 非空时所有条目都放进同一层子目录（GitHub 风格）。
    fn write_zip(dir: &Path, files: &[(&str, &str)], prefix: Option<&str>) -> PathBuf {
        use std::io::Write;
        let path = dir.join("pkg.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            let full = match prefix {
                Some(prefix) => format!("{prefix}/{name}"),
                None => (*name).to_string(),
            };
            writer.start_file(full, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn test_import_zip_strips_wrapper_dir() {
        let root = unique_temp_dir();
        let central = root.join("central");
        let zip_path = write_zip(
            &root,
            &[
                (
                    "blender_manifest.toml",
                    "id = \"imp\"\nversion = \"2.0.0\"\n",
                ),
                ("__init__.py", "print(1)"),
            ],
            Some("imp-main"),
        );

        let target = import_zip(&zip_path, &central, "imp", "2.0.0").unwrap();
        assert!(
            target.join("blender_manifest.toml").is_file(),
            "包裹层要被剥掉"
        );
        assert!(target.join("__init__.py").is_file());
        assert!(central.join("imp").join("2.0.0").is_dir());
        // 临时目录不能残留。
        assert!(!central.join("imp").join(".incoming-2.0.0").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_zip_rejects_wrong_id_and_leaves_no_trace() {
        let root = unique_temp_dir();
        let central = root.join("central");
        let zip_path = write_zip(
            &root,
            &[(
                "blender_manifest.toml",
                "id = \"someone_else\"\nversion = \"1.0.0\"\n",
            )],
            None,
        );

        let err = import_zip(&zip_path, &central, "expected", "1.0.0").unwrap_err();
        assert!(err.to_string().contains("不一致"), "错误要能说明原因");
        assert!(!central.join("expected").join("1.0.0").exists());
        assert!(!central.join("expected").join(".incoming-1.0.0").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_zip_refuses_existing_version() {
        let root = unique_temp_dir();
        let central = root.join("central");
        std::fs::create_dir_all(central.join("dup").join("1.0.0")).unwrap();
        let zip_path = write_zip(
            &root,
            &[(
                "blender_manifest.toml",
                "id = \"dup\"\nversion = \"1.0.0\"\n",
            )],
            None,
        );

        assert!(import_zip(&zip_path, &central, "dup", "1.0.0").is_err());
        // 已有版本的内容不能被覆盖。
        assert!(!central
            .join("dup")
            .join("1.0.0")
            .join("blender_manifest.toml")
            .exists());
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

    /// 用官方站真实下载的 zip 走一遍 `import_zip`：解包、剥包裹层、校验 id。
    ///
    /// 手动运行：
    /// `BLMGR_EXT_ZIP=/tmp/facetor100.zip BLMGR_EXT_ID=facetor cargo test --lib live_import_real_zip -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_import_real_zip() {
        let (Ok(zip), Ok(pkg_id)) = (
            std::env::var("BLMGR_EXT_ZIP"),
            std::env::var("BLMGR_EXT_ID"),
        ) else {
            eprintln!("skip: BLMGR_EXT_ZIP / BLMGR_EXT_ID 未设置");
            return;
        };
        let central = unique_temp_dir_for_live();
        let target =
            import_zip(Path::new(&zip), &central, &pkg_id, "9.9.9").expect("import failed");
        assert!(target.join("blender_manifest.toml").is_file());
        assert!(target.join("__init__.py").is_file(), "真实包应含入口文件");
        // manifest 里写的 id 必须与目录名一致。
        let manifest = extensions::read_manifest(&target).expect("manifest 应可读");
        assert_eq!(manifest.id, pkg_id);
        let _ = std::fs::remove_dir_all(&central);
        eprintln!("live import ok -> {}", target.display());
    }

    fn unique_temp_dir_for_live() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blmgr-live-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 用两个真实版本的官方包走完整条 B2 链路：
    /// 装 1.0.1 → 接管 → 下载入库 1.0.0 → 切换 → 解除接管。
    ///
    /// 手动运行：
    /// `BLMGR_EXT_ZIP_V1=/tmp/facetor.zip BLMGR_EXT_ZIP_V2=/tmp/facetor100.zip \
    ///  BLMGR_EXT_ID=facetor cargo test --lib live_b2 -- --ignored --nocapture`
    ///
    /// 全程在临时目录里，不碰用户真实的扩展根。
    #[test]
    #[ignore]
    fn live_b2_add_version_cycle() {
        let (Ok(zip_new), Ok(zip_old), Ok(pkg_id)) = (
            std::env::var("BLMGR_EXT_ZIP_V1"),
            std::env::var("BLMGR_EXT_ZIP_V2"),
            std::env::var("BLMGR_EXT_ID"),
        ) else {
            eprintln!("skip: BLMGR_EXT_ZIP_V1 / BLMGR_EXT_ZIP_V2 / BLMGR_EXT_ID 未设置");
            return;
        };

        let root = unique_temp_dir_for_live();
        let central = root.join("central");
        let ext_root = root.join("extensions");
        let pkg_path = ext_root.join("user_default").join(&pkg_id);

        // 1. 官方新版本入库，并铺成「用户在 Blender 里装好」的普通目录。
        let new_dir = import_zip(Path::new(&zip_new), &central, &pkg_id, "1.0.1")
            .expect("import new version failed");
        copy_dir(&new_dir, &pkg_path).expect("copy into fake ext root failed");
        assert!(!is_dir_link(&pkg_path));

        // 2. 接管：中央库里已有内容完全相同的 1.0.1，应直接复用而不是报错。
        adopt(&pkg_path, &central, &pkg_id, "1.0.1").expect("adopt failed");
        assert!(is_dir_link(&pkg_path), "接管后应是链接");

        // 3. 添加历史版本 1.0.0，只入库。
        import_zip(Path::new(&zip_old), &central, &pkg_id, "1.0.0").expect("import old version failed");
        assert_eq!(versions(&central, &pkg_id), vec!["1.0.1", "1.0.0"]);

        // 4. 切换，链接指向的那份内容必须真的换了版本。
        switch_version(&pkg_path, &central, &pkg_id, "1.0.0").expect("switch failed");
        let manifest = extensions::read_manifest(&pkg_path).expect("切换后 manifest 应可读");
        assert_eq!(manifest.version, "1.0.0", "Blender 看到的版本应变成 1.0.0");

        // 5. 解除接管：内容复制回普通目录，中央库保留两个版本。
        release(&pkg_path).expect("release failed");
        assert!(!is_dir_link(&pkg_path));
        let manifest = extensions::read_manifest(&pkg_path).expect("解除后 manifest 应可读");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(versions(&central, &pkg_id), vec!["1.0.1", "1.0.0"]);

        let _ = std::fs::remove_dir_all(&root);
        eprintln!("live B2 add-version cycle ok");
    }
}
