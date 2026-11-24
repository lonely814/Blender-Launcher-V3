use crate::service::launcher::{InstanceEntry, LaunchedInstance, Launcher};
use crate::state::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, State};

/// 进程资源采样间隔；也决定退出检测的轮询频率。
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// 启动 Blender (docs/01 §3.3)。返回 instance id（= PID）。
#[tauri::command]
pub async fn launch_blender(
    build_id: String,
    blend_file: Option<PathBuf>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    open_last: Option<bool>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    launch_impl(&state, &app_handle, build_id, blend_file, args, env, open_last.unwrap_or(false)).await
}

/// 命令与托盘共用的启动实现（State 不能跨 await 持有，故接收引用）。
pub async fn launch_impl(
    state: &AppState,
    app_handle: &tauri::AppHandle,
    build_id: String,
    blend_file: Option<PathBuf>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    open_last: bool,
) -> Result<String, String> {
    let (build, default_args) = {
        let builds = state.builds.lock().unwrap();
        let settings = state.settings.lock().unwrap();
        match builds.iter().find(|b| b.id == build_id) {
            Some(build) => (build.clone(), settings.launch_args.clone()),
            None => return Err(format!("Build not found: {build_id}")),
        }
    };

    // 最近项目：启动成功后记录（docs/01 §3.3）。
    let recent_file = blend_file
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let recent_build = build.id.clone();

    let entry = Launcher::launch(&build, &default_args, blend_file, args, env, open_last)
        .await
        .map_err(|err| err.to_string())?;

    if let (Some(file), Some(db)) = (recent_file, state.db.as_ref().map(Arc::clone)) {
        tokio::task::spawn_blocking(move || db.record_recent_project(&file, &recent_build))
            .await
            .ok();
    }

    let instance_id = entry.info.id.clone();

    // 先入表再启动监控，保证监控回调一定能找到实例。
    state
        .running_instances
        .lock()
        .unwrap()
        .insert(instance_id.clone(), entry.clone());

    // 发 instance_started 事件（docs/01 §4）。
    let _ = app_handle.emit("instance_started", &entry.info);

    // 进程监控：轮询 try_wait 检测退出（docs/00 §1.2 进程管理 + 崩溃检测），
    // 每 2s 用 sysinfo 采样内存/CPU 更新 LaunchedInstance（docs/01 §2.3）。
    // 退出后移除实例并发 instance_exited { instanceId, exitCode }（docs/01 §4）。
    let running = Arc::clone(&state.running_instances);
    let monitor_entry = entry;
    let monitor_app = app_handle.clone();
    let monitor_id = instance_id.clone();
    tokio::spawn(async move {
        use sysinfo::{Pid, ProcessesToUpdate, System};

        let pid = Pid::from_u32(monitor_id.parse().unwrap_or(0));
        let mut sys = System::new();
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);

        let exit_code = loop {
            {
                let mut child = monitor_entry.child.lock().await;
                match child.try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(instance_id = %monitor_id, error = %err, "failed to poll blender process");
                        break None;
                    }
                }
            }

            interval.tick().await;
            sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
            if let Some(process) = sys.process(pid) {
                if let Some(entry) = running.lock().unwrap().get_mut(&monitor_id) {
                    entry.info.memory_mb = process.memory() / 1024 / 1024;
                    entry.info.cpu_percent = process.cpu_usage();
                }
            }
        };

        running.lock().unwrap().remove(&monitor_id);
        match exit_code {
            Some(0) => tracing::info!(instance_id = %monitor_id, "blender exited normally"),
            Some(code) => {
                // 崩溃检测（docs/00 §1.2）：非零退出码。
                tracing::warn!(instance_id = %monitor_id, exit_code = code, "blender crashed or exited with an error");
            }
            None => {
                tracing::warn!(instance_id = %monitor_id, "blender exited without an exit code (killed or signaled)")
            }
        }
        let _ = monitor_app.emit(
            "instance_exited",
            serde_json::json!({ "instanceId": monitor_id, "exitCode": exit_code }),
        );
    });

    Ok(instance_id)
}

/// 获取运行中的 Blender 实例 (docs/01 §3.3)。
#[tauri::command]
pub async fn get_running_instances(
    state: State<'_, AppState>,
) -> Result<Vec<LaunchedInstance>, String> {
    let instances = state.running_instances.lock().unwrap();
    Ok(instances.values().map(|entry| entry.info.clone()).collect())
}

/// 强制关闭实例 (docs/01 §3.3)。
#[tauri::command]
pub async fn kill_instance(instance_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let entry: Option<InstanceEntry> = {
        let instances = state.running_instances.lock().unwrap();
        instances.get(&instance_id).cloned()
    };
    match entry {
        Some(entry) => {
            let mut child = entry.child.lock().await;
            child
                .kill()
                .await
                .map_err(|err| format!("Failed to kill instance {instance_id}: {err}"))?;
            // 实例的移除和 instance_exited 事件由监控任务的 wait() 收尾。
            Ok(())
        }
        None => Err(format!("Instance not found: {instance_id}")),
    }
}
