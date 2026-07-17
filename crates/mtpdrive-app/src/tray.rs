use crate::tray_template;
use mtpdrive_core::Language;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent, TrayIconId,
};

const MENU_SHOW: &str = "mtpdrive.show";
const MENU_OPEN: &str = "mtpdrive.open";
const MENU_REFRESH: &str = "mtpdrive.refresh";
const MENU_QUIT: &str = "mtpdrive.quit";
const TRAY_ID: &str = "mtpdrive.main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Show,
    OpenFinder,
    RefreshDevices,
    Quit,
}

pub(crate) struct Tray {
    icon: TrayIcon,
}

impl Tray {
    pub(crate) fn new(language: Language) -> Result<Self, String> {
        let menu = create_menu(language)?;
        let icon = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_tooltip("MTPDrive")
            .with_icon(template_icon()?)
            .with_icon_as_template(true)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { icon })
    }

    pub(crate) fn set_language(&self, language: Language) -> Result<(), String> {
        self.icon.set_menu(Some(Box::new(create_menu(language)?)));
        Ok(())
    }

    pub(crate) fn drain_actions(active: Option<&Self>) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if is_active_tray_id(active.map(|tray| tray.icon.id()), event.id())
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                )
            {
                actions.push(TrayAction::Show);
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = menu_action(event.id.0.as_str());
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }
}

fn is_active_tray_id(active: Option<&TrayIconId>, event: &TrayIconId) -> bool {
    active.is_some_and(|active| active == event)
}

fn menu_action(id: &str) -> Option<TrayAction> {
    match id {
        MENU_SHOW => Some(TrayAction::Show),
        MENU_OPEN => Some(TrayAction::OpenFinder),
        MENU_REFRESH => Some(TrayAction::RefreshDevices),
        MENU_QUIT => Some(TrayAction::Quit),
        _ => None,
    }
}

fn create_menu(language: Language) -> Result<Menu, String> {
    let strings = language.strings();
    let show = MenuItem::with_id(MENU_SHOW, strings.show_mtpdrive, true, None);
    let open = MenuItem::with_id(MENU_OPEN, strings.open_in_finder, true, None);
    let refresh = MenuItem::with_id(MENU_REFRESH, strings.rescan_devices, true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(MENU_QUIT, strings.quit_mtpdrive, true, None);
    Menu::with_items(&[&show, &open, &refresh, &separator, &quit])
        .map_err(|error| error.to_string())
}

fn template_icon() -> Result<Icon, String> {
    Icon::from_rgba(
        tray_template::rgba(),
        tray_template::SIZE,
        tray_template::SIZE,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "../tests/unit/tray.rs"]
mod tests;
