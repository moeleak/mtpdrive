use super::progress_ratio;
use crate::application::{App, AppearanceOption, LanguageOption, Message, UpdateState};
use crate::login_item;
use iced::Length;
use iced::widget::{Column, Row};
use material::widget::{button, container, page, progress_bar, select, toggler};
use material_ui_rs as material;

pub(super) fn view(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let finder_behavior = Column::new()
        .push(toggler::standard(
            app.settings.always_open_in_finder,
            strings.always_open_in_finder,
            Message::AlwaysOpenChanged,
        ))
        .push(material::text::body_medium(
            strings.always_open_in_finder_description,
        ))
        .spacing(8);
    let mut login_behavior = Column::new()
        .push(toggler::standard(
            app.login_item_status.is_registered(),
            strings.open_at_login,
            Message::OpenAtLoginChanged,
        ))
        .push(material::text::body_medium(
            strings.open_at_login_description,
        ))
        .spacing(8);
    match app.login_item_status {
        login_item::Status::RequiresApproval => {
            login_behavior = login_behavior
                .push(material::text::body_medium(
                    strings.open_at_login_requires_approval,
                ))
                .push(
                    button::button(
                        strings.open_login_items_settings,
                        button::ButtonVariant::Text,
                    )
                    .on_press(Message::OpenLoginItemsSettings),
                );
        }
        #[cfg(not(target_os = "macos"))]
        login_item::Status::Unavailable => {
            login_behavior = login_behavior.push(material::text::body_medium(
                strings.open_at_login_unavailable,
            ));
        }
        login_item::Status::Disabled | login_item::Status::Enabled => {}
    }
    if let Some(error) = &app.login_item_error {
        login_behavior = login_behavior.push(material::text::body_medium(
            app.language.open_at_login_failed(error),
        ));
    }
    let behavior_content = Column::new()
        .push(finder_behavior)
        .push(login_behavior)
        .spacing(20);
    let behavior = container::outlined_card(behavior_content)
        .padding(20)
        .width(Length::Fill);

    let selected_language = app
        .language_options
        .iter()
        .find(|option| option.preference == app.settings.language);
    let language_picker = select::outlined(
        app.language_options.as_slice(),
        selected_language,
        |option: LanguageOption| Message::LanguageChanged(option.preference),
    )
    .label(strings.language)
    .width(Length::Fixed(320.0));
    let language = container::outlined_card(
        Column::new()
            .push(material::text::title_medium(strings.language))
            .push(material::text::body_medium(strings.language_description))
            .push(language_picker)
            .spacing(10),
    )
    .padding(20)
    .width(Length::Fill);

    let selected_appearance = app
        .appearance_options
        .iter()
        .find(|option| option.preference == app.settings.appearance);
    let appearance_picker = select::outlined(
        app.appearance_options.as_slice(),
        selected_appearance,
        |option: AppearanceOption| Message::AppearanceChanged(option.preference),
    )
    .label(strings.appearance)
    .width(Length::Fixed(320.0));
    let theme = container::outlined_card(
        Column::new()
            .push(material::text::title_medium(strings.theme))
            .push(material::text::body_medium(strings.theme_description))
            .push(appearance_picker)
            .push(material::text::body_medium(strings.theme_picker_hint))
            .spacing(10),
    )
    .padding(20)
    .width(Length::Fill);

    let mut update = Column::new()
        .push(material::text::title_medium(strings.about))
        .push(material::text::body_large(
            app.language.current_version(env!("CARGO_PKG_VERSION")),
        ))
        .spacing(10);
    update = match &app.update_state {
        UpdateState::Idle => update.push(
            button::button(
                strings.check_for_updates,
                button::ButtonVariant::FilledTonal,
            )
            .on_press(Message::CheckForUpdates),
        ),
        UpdateState::Checking => update.push(
            Row::new()
                .push(progress_bar::loading(
                    progress_bar::LoadingIndicatorMode::contained_indeterminate(
                        app.progress_animation.loading_phase(),
                    ),
                ))
                .push(material::text::body_medium(strings.checking_for_updates))
                .spacing(12)
                .align_y(iced::Alignment::Center),
        ),
        UpdateState::UpToDate => update
            .push(material::text::body_medium(strings.up_to_date))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::CheckFailed(error) => update
            .push(material::text::body_medium(
                app.language.update_check_failed(error),
            ))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::Downloading {
            asset, downloaded, ..
        } => {
            let progress = if asset.size == 0 {
                0.0
            } else {
                progress_ratio(*downloaded, asset.size)
            };
            update
                .push(material::text::body_medium(
                    app.language
                        .downloading_update(*downloaded, asset.size, progress),
                ))
                .push(progress_bar::linear(
                    progress_bar::LinearProgressMode::determinate(
                        progress,
                        app.progress_animation.linear_phase(),
                    ),
                ))
        }
        UpdateState::Verifying { .. } => update.push(
            Row::new()
                .push(progress_bar::loading(
                    progress_bar::LoadingIndicatorMode::contained_indeterminate(
                        app.progress_animation.loading_phase(),
                    ),
                ))
                .push(material::text::body_medium(strings.verifying_update))
                .spacing(12)
                .align_y(iced::Alignment::Center),
        ),
        UpdateState::Downloaded(path) => update
            .push(material::text::body_medium(
                app.language.update_downloaded(path.display()),
            ))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::DownloadFailed {
            version,
            asset,
            error,
        } => update
            .push(material::text::body_medium(
                app.language.update_download_failed(error),
            ))
            .push(
                button::button(strings.retry_download, button::ButtonVariant::FilledTonal)
                    .on_press(Message::DownloadUpdate {
                        version: version.clone(),
                        asset: asset.clone(),
                    }),
            ),
    };
    let about = container::outlined_card(update)
        .padding(20)
        .width(Length::Fill);

    page::surface(
        page::header(strings.settings, strings.settings_description),
        Column::new()
            .push(behavior)
            .push(theme)
            .push(language)
            .push(about)
            .spacing(16)
            .width(Length::Fill),
    )
    .into()
}
