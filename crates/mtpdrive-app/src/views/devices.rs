use super::progress_ratio;
use crate::application::{App, Message};
use iced::Length;
use iced::widget::{Column, Row, Space};
use material::widget::{button, container, page, progress_bar};
use material_ui_rs as material;
use mtpdrive_core::{DeviceSummary, Language, MountState, format_bytes, format_mount_state};

pub(super) fn view(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let mount_description = format_mount_state(app.language, &app.snapshot.mount);

    let mut action_items: Vec<material::Element<'_, Message>> = Vec::with_capacity(2);
    if !matches!(app.snapshot.mount, MountState::Mounted { .. }) {
        action_items.push(
            button::button(strings.mount, button::ButtonVariant::Filled)
                .on_press(Message::Mount)
                .into(),
        );
    }
    action_items.push(
        button::button(strings.open_in_finder, button::ButtonVariant::FilledTonal)
            .on_press(Message::OpenFinder)
            .into(),
    );
    let actions = page::row(action_items);

    let status = container::filled_card(
        Column::new()
            .push(material::text::headline_medium(strings.network_volume))
            .push(material::text::body_large(mount_description))
            .push(actions)
            .spacing(12),
    )
    .padding(18)
    .width(Length::Fill);

    let mut body = Column::new().push(status).spacing(16).width(Length::Fill);
    if let Some(error) = &app.error {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::title_medium(strings.action_required))
                    .push(material::text::body_medium(error)),
            )
            .padding(16)
            .width(Length::Fill),
        );
    }
    if let Some(error) = &app.snapshot.last_error {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::title_medium(strings.device_action_required))
                    .push(material::text::body_medium(error)),
            )
            .padding(16)
            .width(Length::Fill),
        );
    }
    if app.snapshot.devices.is_empty() {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::headline_medium(strings.no_devices))
                    .push(material::text::body_large(strings.connect_android))
                    .spacing(6),
            )
            .padding(22)
            .width(Length::Fill),
        );
    } else {
        let progress_phase = app.progress_animation.linear_phase();
        for device in &app.snapshot.devices {
            body = body.push(device_card(app.language, device, progress_phase));
        }
    }

    page::surface(
        page::header(
            strings.devices,
            app.language.device_count(app.snapshot.devices.len()),
        ),
        body,
    )
    .into()
}

fn device_card(
    language: Language,
    device: &DeviceSummary,
    progress_phase: f32,
) -> material::Element<'_, Message> {
    let strings = language.strings();
    let mut content = Column::new()
        .push(material::text::headline_medium(format!(
            "{} {}",
            device.manufacturer, device.model
        )))
        .push(material::text::body_medium(language.device_details(
            &device.serial,
            device.usb_speed.as_deref().unwrap_or(strings.unknown),
            device.writable,
        )))
        .spacing(8)
        .width(Length::Fill);

    for storage in &device.storages {
        let used = storage.total_bytes.saturating_sub(storage.free_bytes);
        let ratio = if storage.total_bytes == 0 {
            0.0
        } else {
            progress_ratio(used, storage.total_bytes)
        };
        content = content.push(
            Column::new()
                .push(
                    Row::new()
                        .push(material::text::title_medium(&storage.name))
                        .push(Space::new().width(Length::Fill))
                        .push(material::text::body_medium(language.storage_capacity(
                            &format_bytes(storage.free_bytes),
                            &format_bytes(storage.total_bytes),
                        ))),
                )
                .push(progress_bar::linear(
                    progress_bar::LinearProgressMode::determinate(ratio, progress_phase),
                ))
                .spacing(6),
        );
    }
    container::outlined_card(content)
        .padding(20)
        .width(Length::Fill)
        .into()
}
