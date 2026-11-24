use crate::error::AppError;
use crate::Result;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub struct Extractor;

impl Extractor {
    /// 解压归档到 `dest_dir`（docs/03 目录结构 service/extractor.rs）。
    ///
    /// 支持下载产物中的可解压格式：`.zip`（Windows）、`.tar.xz`
    /// （Linux/macOS）。`.msi`/`.dmg` 是安装器格式，不解压，返回 `false`
    /// 由调用方保留安装包并直接标记完成。
    ///
    /// 成功解压后由调用方决定是否删除归档。路径穿越（zip-slip）由
    /// zip/tar crate 的 extract/unpack 内建防护处理。
    pub async fn extract(archive: PathBuf, dest_dir: PathBuf) -> Result<bool> {
        // CPU/IO 密集的同步操作放到阻塞线程池，不卡 async 运行时。
        tokio::task::spawn_blocking(move || Self::extract_sync(&archive, &dest_dir))
            .await
            .map_err(|err| AppError::ExtractFailed(err.to_string()))?
    }

    fn extract_sync(archive: &Path, dest_dir: &Path) -> Result<bool> {
        let name = archive
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if name.ends_with(".zip") {
            Self::extract_zip(archive, dest_dir)?;
            return Ok(true);
        }
        if name.ends_with(".tar.xz") {
            Self::extract_tar(archive, dest_dir, xz_decode)?;
            return Ok(true);
        }
        if name.ends_with(".tar.gz") {
            Self::extract_tar(archive, dest_dir, gz_decode)?;
            return Ok(true);
        }
        Ok(false) // .msi / .dmg / .exe：安装器格式，不解压
    }

    /// `.tar.*` 通用解包：先把压缩流转码为临时 .tar，再 tar 解包
    /// （纯 Rust 实现，Windows 上不依赖系统工具）。
    fn extract_tar(
        archive: &Path,
        dest_dir: &Path,
        decode: fn(&Path, &Path) -> Result<()>,
    ) -> Result<()> {
        let tar_path = archive.with_extension("tar");
        decode(archive, &tar_path)?;

        let tar_file = std::fs::File::open(&tar_path).map_err(AppError::from)?;
        tar::Archive::new(BufReader::new(tar_file))
            .unpack(dest_dir)
            .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
        std::fs::remove_file(&tar_path).map_err(AppError::from)?;
        Ok(())
    }

    fn extract_zip(archive: &Path, dest_dir: &Path) -> Result<()> {
        let file = std::fs::File::open(archive).map_err(AppError::from)?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|err| AppError::ExtractFailed(err.to_string()))?;
        zip.extract(dest_dir)
            .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
        Ok(())
    }
}

/// xz 流解码为 .tar（lzma-rs，纯 Rust）。
fn xz_decode(archive: &Path, tar_path: &Path) -> Result<()> {
    let input = BufReader::new(std::fs::File::open(archive).map_err(AppError::from)?);
    let mut output = std::fs::File::create(tar_path).map_err(AppError::from)?;
    lzma_rs::xz_decompress(&mut { input }, &mut output)
        .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
    Ok(())
}

/// gzip 流解码为 .tar（flate2，纯 Rust）。
fn gz_decode(archive: &Path, tar_path: &Path) -> Result<()> {
    let input = BufReader::new(std::fs::File::open(archive).map_err(AppError::from)?);
    let mut output = std::fs::File::create(tar_path).map_err(AppError::from)?;
    let mut decoder = flate2::read::GzDecoder::new(input);
    std::io::copy(&mut decoder, &mut output)
        .map_err(|err| AppError::ExtractFailed(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bl-extract-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_extract_zip() {
        let archive_dir = temp_dir("zip-src");
        let dest = temp_dir("zip-out");
        let archive = archive_dir.join("blender-4.5.0-windows-x64.zip");

        // 用 zip crate 生成一个真实归档。
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "blender-4.5.0-windows-x64/blender.exe",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"fake blender binary").unwrap();
        zip.finish().unwrap();

        let extracted = Extractor::extract(archive.clone(), dest.clone())
            .await
            .unwrap();
        assert!(extracted);
        assert!(dest.join("blender-4.5.0-windows-x64/blender.exe").exists());
    }

    #[tokio::test]
    async fn test_extract_tar_xz() {
        let archive_dir = temp_dir("tar-src");
        let dest = temp_dir("tar-out");
        let archive = archive_dir.join("blender-4.5.0-linux-x64.tar.xz");

        // tar 打包 → xz 压缩，生成真实归档。
        let tar_path = archive.with_extension("tar");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let mut builder = tar::Builder::new(file);
            let content = b"fake blender binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "blender-4.5.0-linux-x64/blender", &content[..])
                .unwrap();
            builder.finish().unwrap();
        }
        {
            let tar_input = BufReader::new(std::fs::File::open(&tar_path).unwrap());
            let mut xz_output = std::fs::File::create(&archive).unwrap();
            lzma_rs::xz_compress(&mut { tar_input }, &mut xz_output).unwrap();
        }
        std::fs::remove_file(&tar_path).unwrap();

        let extracted = Extractor::extract(archive, dest.clone()).await.unwrap();
        assert!(extracted);
        assert!(dest.join("blender-4.5.0-linux-x64/blender").exists());
    }

    #[tokio::test]
    async fn test_installer_formats_are_skipped() {
        let dir = temp_dir("msi-skip");
        let archive = dir.join("blender-4.5.0-windows-x64.msi");
        std::fs::write(&archive, b"fake msi").unwrap();

        let extracted = Extractor::extract(archive, dir.clone()).await.unwrap();
        assert!(!extracted, "msi must not be extracted");
    }
}
