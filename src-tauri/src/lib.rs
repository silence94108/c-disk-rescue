mod cleaner;
mod disk;
mod rules;
mod scan;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(scan::ScanState::default())
        .manage(cleaner::CleanState::default())
        .invoke_handler(tauri::generate_handler![
            disk::get_disks,
            scan::start_scan,
            scan::cancel_scan,
            scan::get_children,
            scan::get_migratables,
            cleaner::scan_cleanables,
            cleaner::check_locks,
            cleaner::run_clean,
            cleaner::cancel_clean,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
