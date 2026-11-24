use crate::error::AppError;
use crate::models::Build;
use crate::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A running Blender instance (docs/01 §2.3).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchedInstance {
    /// The OS process id, used as the instance id.
    pub id: String,
    pub build_id: String,
    pub blend_file: Option<String>,
    pub started_at: DateTime<Utc>,
    /// Memory usage in MB. 0 until live sampling lands (needs `sysinfo`).
    pub memory_mb: u64,
    /// CPU usage in percent. 0 until live sampling lands (needs `sysinfo`).
    pub cpu_percent: f32,
}

/// Live handle kept in state so `kill_instance` can reach the process and
/// the monitor task can detect exit (docs/00 §1.2: 进程管理 + 崩溃检测).
#[derive(Clone)]
pub struct InstanceEntry {
    pub info: LaunchedInstance,
    pub child: Arc<Mutex<tokio::process::Child>>,
}

pub struct Launcher;

impl Launcher {
    /// Starts Blender for an installed build.
    ///
    /// Command line order: settings default args → user args → .blend file
    /// (docs/01 §3.3 `launch_blender`).
    #[allow(clippy::too_many_arguments)]
    pub async fn launch(
        build: &Build,
        default_args: &[String],
        blend_file: Option<PathBuf>,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
        open_last: bool,
    ) -> Result<InstanceEntry> {
        let executable = Self::find_executable(build)?;
        let command_args = Self::build_command_args(
            default_args,
            args.as_deref(),
            blend_file.as_deref(),
            open_last,
        );

        let mut cmd = tokio::process::Command::new(&executable);
        cmd.args(&command_args);
        if let Some(env) = env.as_ref() {
            for (key, value) in env {
                cmd.env(key, value);
            }
        }

        let child = cmd
            .spawn()
            .map_err(|err| AppError::LaunchFailed(err.to_string()))?;

        let pid = child
            .id()
            .ok_or_else(|| AppError::LaunchFailed("process exited before PID was read".into()))?;

        let info = LaunchedInstance {
            id: pid.to_string(),
            build_id: build.id.clone(),
            blend_file: blend_file.map(|file| file.to_string_lossy().into_owned()),
            started_at: Utc::now(),
            memory_mb: 0,
            cpu_percent: 0.0,
        };

        let child = Arc::new(Mutex::new(child));
        Ok(InstanceEntry { info, child })
    }

    /// Assembles the blender command line (pure, unit-tested).
    fn build_command_args(
        default_args: &[String],
        user_args: Option<&[String]>,
        blend_file: Option<&Path>,
        open_last: bool,
    ) -> Vec<String> {
        let mut args: Vec<String> = default_args.to_vec();
        if let Some(user_args) = user_args {
            args.extend(user_args.iter().cloned());
        }
        // 原版 Shift+双击 / Open Previous File → --open-last
        if open_last {
            args.push("--open-last".to_string());
        }
        if let Some(file) = blend_file {
            args.push(file.to_string_lossy().into_owned());
        }
        args
    }

    /// Locates the blender executable for the current platform.
    /// 原版优先级：custom_executable → bforartists.* → blender.*
    fn find_executable(build: &Build) -> Result<PathBuf> {
        let install_path = build
            .install_path
            .as_ref()
            .ok_or_else(|| AppError::BuildNotFound("Not installed".into()))?;

        #[cfg(target_os = "windows")]
        let candidates = ["bforartists.exe", "blender.exe"];
        #[cfg(target_os = "macos")]
        let candidates = [
            "Bforartists.app/Contents/MacOS/Bforartists",
            "Blender.app/Contents/MacOS/Blender",
        ];
        #[cfg(target_os = "linux")]
        let candidates = ["bforartists", "blender"];

        for name in candidates {
            let exe = install_path.join(name);
            if exe.exists() {
                return Ok(exe);
            }
        }
        Err(AppError::BuildNotFound(format!(
            "Executable not found in {install_path:?} (tried {candidates:?})"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_args_order() {
        let args = Launcher::build_command_args(
            &["--factory-startup".to_string()],
            Some(["--debug".to_string()].as_slice()),
            Some(Path::new("C:/scenes/test.blend")),
            true,
        );
        assert_eq!(
            args,
            vec![
                "--factory-startup".to_string(),
                "--debug".to_string(),
                "--open-last".to_string(),
                "C:/scenes/test.blend".to_string()
            ]
        );
    }

    #[test]
    fn test_build_command_args_empty() {
        assert!(Launcher::build_command_args(&[], None, None, false).is_empty());
        assert_eq!(
            Launcher::build_command_args(&[], None, Some(Path::new("/tmp/a.blend")), false),
            vec!["/tmp/a.blend".to_string()]
        );
        assert_eq!(
            Launcher::build_command_args(&[], None, None, true),
            vec!["--open-last".to_string()]
        );
    }
}
