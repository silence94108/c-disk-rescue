mod cleaner;
mod disk;
mod migrator;
mod rules;
mod scan;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(scan::ScanState::default())
        .manage(cleaner::CleanState::default())
        .manage(migrator::MigrateState::default())
        .invoke_handler(tauri::generate_handler![
            disk::get_disks,
            scan::start_scan,
            scan::cancel_scan,
            scan::get_children,
            scan::get_migratables,
            scan::get_capacity_breakdown,
            scan::get_migrate_candidates,
            scan::get_big_files,
            scan::delete_big_file,
            scan::get_orphan_profiles,
            scan::delete_orphan_profile,
            cleaner::scan_cleanables,
            cleaner::check_locks,
            cleaner::run_clean,
            cleaner::cancel_clean,
            migrator::get_migrate_targets,
            migrator::start_migrate,
            migrator::cancel_migrate,
            migrator::get_migrations,
            migrator::confirm_migration,
            migrator::revert_migration,
            migrator::request_close,
            migrator::recover_pending_migration,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
