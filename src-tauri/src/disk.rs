use serde::Serialize;
use sysinfo::Disks;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    /// 挂载点,如 "C:\\"
    pub mount_point: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub is_removable: bool,
    pub is_system: bool,
}

#[tauri::command]
pub fn get_disks() -> Vec<DiskInfo> {
    let system_root = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    Disks::new_with_refreshed_list()
        .iter()
        .map(|d| {
            let mount = d.mount_point().to_string_lossy().to_string();
            DiskInfo {
                is_system: mount.to_uppercase().starts_with(&system_root.to_uppercase()),
                total_bytes: d.total_space(),
                free_bytes: d.available_space(),
                is_removable: d.is_removable(),
                mount_point: mount,
            }
        })
        .collect()
}
