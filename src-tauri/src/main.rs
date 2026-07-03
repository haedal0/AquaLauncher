//! AquaLauncher 엔트리포인트.
//! TODO(PRD 8.13): tracing 파일 로거 초기화 + panic hook

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// 골격 단계: 스텁 trait/타입의 dead_code 경고 억제. 각 모듈 구현 착수 시 해당 allow 제거.
#![allow(dead_code)]

mod auth;
mod browse;
mod cache;
mod commands;
mod deeplink;
mod error;
mod launch;
mod loaders;
mod net;
mod store;
mod sync;

use tauri::Manager;

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

fn main() {
    tauri::Builder::default()
        // 싱글 인스턴스 보장 (§8.7) — 반드시 첫 플러그인으로 등록.
        // deep-link 피처가 2차 실행의 딥링크를 on_open_url로 중계하므로 여기선 포커스만.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            // 딥링크(§8.7): 검증 통과한 매니페스트 URL만 프론트 확인 모달로 전달
            use tauri_plugin_deep_link::DeepLinkExt;
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                use tauri::Emitter;
                for url in event.urls() {
                    match deeplink::parse_add_link(url.as_str()) {
                        Ok(manifest_url) => {
                            let _ = handle.emit("deeplink-add", manifest_url);
                        }
                        Err(e) => tracing::warn!(%e, "deep link rejected"),
                    }
                }
            });
            let data_root = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            std::fs::create_dir_all(&data_root)?;
            app.manage(commands::AppState {
                paths: store::Paths { data_root },
            });
            // 시작 시 잔존 커밋 저널 복구 — TD-02 §4.3
            let state: tauri::State<commands::AppState> = app.state();
            for cfg in store::list_instances(&state.paths) {
                let dir = state.paths.instance_dir(&cfg.id);
                match sync::commit::recover(&dir) {
                    Ok(sync::commit::Recovery::Clean) => {}
                    Ok(outcome) => tracing::info!(instance = %cfg.id, ?outcome, "staging recovery"),
                    Err(e) => tracing::warn!(instance = %cfg.id, %e, "staging recovery failed"),
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::list_instances,
            commands::create_manual_instance,
            commands::preview_manifest,
            commands::create_instance_from_manifest,
            commands::toggle_mod,
            commands::reset_instance,
            commands::sync_now,
            commands::play,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AquaLauncher");
}
