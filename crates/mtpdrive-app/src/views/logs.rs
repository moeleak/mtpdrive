use crate::application::{App, Message};
use iced::Length;
use iced::widget::{Column, Container, Row, Space};
use material::widget::{button, log_viewer};
use material_ui_rs as material;

pub(super) fn view(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let toolbar = Row::new()
        .push(
            Column::new()
                .push(material::text::headline_large(strings.logs))
                .push(material::text::body_large(
                    app.language.recent_log_count(app.log_entries.len()),
                ))
                .spacing(4),
        )
        .push(Space::new().width(Length::Fill))
        .push(
            button::button(strings.clear_view, button::ButtonVariant::Text)
                .on_press(Message::ClearLogView),
        )
        .align_y(iced::Alignment::Center);
    let viewer = log_viewer::view(&app.log_entries, &app.log_viewer, Message::LogViewer)
        .width(Length::Fill)
        .height(Length::Fill);
    Container::new(Column::new().push(toolbar).push(viewer).spacing(18))
        .padding(28)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
