use super::{
    MENU_OPEN, MENU_QUIT, MENU_REFRESH, MENU_SHOW, TRAY_ID, TrayAction, is_active_tray_id,
    menu_action,
};
use tray_icon::TrayIconId;

#[test]
fn tray_id_is_stable_and_filters_foreign_events() {
    assert_eq!(TRAY_ID, "mtpdrive.main");
    let active = TrayIconId::new(TRAY_ID);
    let foreign = TrayIconId::new("another.icon");
    assert!(is_active_tray_id(Some(&active), &active));
    assert!(!is_active_tray_id(Some(&active), &foreign));
    assert!(!is_active_tray_id(None, &active));
}

#[test]
fn menu_ids_map_only_to_mtpdrive_actions() {
    assert_eq!(menu_action(MENU_SHOW), Some(TrayAction::Show));
    assert_eq!(menu_action(MENU_OPEN), Some(TrayAction::OpenFinder));
    assert_eq!(menu_action(MENU_REFRESH), Some(TrayAction::RefreshDevices));
    assert_eq!(menu_action(MENU_QUIT), Some(TrayAction::Quit));
    assert_eq!(menu_action("foreign.menu"), None);
}
