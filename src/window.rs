use std::path::PathBuf;

use cosmic::app::Core;
use cosmic::iced::{
    advanced::text::{Ellipsize, EllipsizeHeightLimit, Wrapping},
    window,
    window::Id,
    ContentFit, Length, Subscription,
};
use cosmic::widget::{
    button, container, icon, image, mouse_area, slider, text, Column, Row, Space,
};
use cosmic::{Action, Element, Task};
use mpris::LoopStatus;

use crate::coordinator;
use crate::metadata::{now_playing_from_player_with_sources, now_playing_snapshot, NowPlayingData};
use crate::player::{
    cycle_loop_status, player_icon_name, select_player, toggle_shuffle, with_player,
};

const ID: &str = "com.github.DiegoMMR.CosmicExtAppletNowPlaying";

#[derive(Default)]
pub struct Window {
    core: Core,
    popup: Option<Id>,
    now_playing_text: String,
    now_playing_title: String,
    now_playing_artist: String,
    now_playing_album: String,
    player_bus_name: String,
    sources: Vec<(String, String)>,
    duration_seconds: Option<u64>,
    position_seconds: Option<u64>,
    can_seek: bool,
    can_go_previous: bool,
    can_go_next: bool,
    can_shuffle: bool,
    shuffle: bool,
    can_loop: bool,
    loop_status: Option<LoopStatus>,
    playback_state: PlaybackState,
    album_art_path: Option<PathBuf>,
    album_art_dimensions: Option<(u32, u32)>,
    has_active_media: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    #[default]
    Unknown,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup {
        anchor_x: i32,
        anchor_y: i32,
        anchor_width: i32,
        anchor_height: i32,
    },
    PopupClosed(Id),
    NowPlayingChanged(NowPlayingData),
    PreviousTrack,
    TogglePlayPause,
    NextTrack,
    SelectSource(String),
    ToggleShuffle,
    CycleLoop,
    Seek(i64),
    SeekTo(u64),
}

impl cosmic::Application for Window {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // Applets must explicitly opt into COSMIC's applet style. Without
        // this, Iced falls back to its default blue controls instead of the
        // user's configured COSMIC accent and surface colours.
        Some(cosmic::applet::style())
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let initial = now_playing_snapshot(false);

        let mut window = Window {
            core,
            ..Default::default()
        };
        window.apply_now_playing(initial);

        (window, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn update(&mut self, message: Message) -> Task<Action<Self::Message>> {
        match message {
            Message::TogglePopup {
                anchor_x,
                anchor_y,
                anchor_width,
                anchor_height,
            } => {
                return if let Some(popup_id) = self.popup.take() {
                    coordinator::set_popup_open(false);
                    cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(popup_id))
                } else {
                    cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                        |_| Default::default(),
                        move |app: &mut Self| {
                            coordinator::set_popup_open(true);
                            let new_id = Id::unique();
                            app.popup.replace(new_id);
                            let mut settings = app.core.applet.get_popup_settings(
                                app.core.main_window_id().unwrap(),
                                new_id,
                                None,
                                None,
                                None,
                            );
                            // `get_popup_settings` uses an icon-sized anchor by
                            // default. This applet is a wide media control, so
                            // anchor to the actual clicked card instead.
                            settings.positioner.anchor_rect = cosmic::iced::Rectangle {
                                x: anchor_x,
                                y: anchor_y,
                                width: anchor_width,
                                height: anchor_height,
                            };
                            settings
                        },
                        None,
                    ))
                };
            }
            Message::PopupClosed(popup_id) => {
                if self.popup.as_ref() == Some(&popup_id) {
                    self.popup = None;
                    coordinator::set_popup_open(false);
                }
            }
            Message::NowPlayingChanged(data) => {
                self.apply_now_playing(data);
            }
            Message::PreviousTrack => {
                if matches!(
                    with_player(&self.player_bus_name, |player| player.previous()),
                    Some(Ok(()))
                ) {
                    coordinator::request_refresh();
                }
            }
            Message::TogglePlayPause => {
                if matches!(
                    with_player(&self.player_bus_name, |player| player.play_pause()),
                    Some(Ok(()))
                ) {
                    // The coordinator will verify this shortly, but updating
                    // immediately makes the control feel responsive without
                    // increasing the background polling rate.
                    self.playback_state = match self.playback_state {
                        PlaybackState::Playing => PlaybackState::Paused,
                        PlaybackState::Paused | PlaybackState::Stopped | PlaybackState::Unknown => {
                            PlaybackState::Playing
                        }
                    };
                    coordinator::request_refresh();
                }
            }
            Message::NextTrack => {
                if matches!(
                    with_player(&self.player_bus_name, |player| player.next()),
                    Some(Ok(()))
                ) {
                    coordinator::request_refresh();
                }
            }
            Message::SelectSource(bus_name) => {
                select_player(bus_name.clone());
                // Selection is applied immediately; the periodic MPRIS refresh
                // will reconcile metadata and capabilities on the next tick.
                let _ = with_player(&bus_name, |player| {
                    let data =
                        now_playing_from_player_with_sources(player, self.sources.clone(), true);
                    self.apply_now_playing(data);
                });
            }
            Message::ToggleShuffle => {
                if let Some(shuffle) = with_player(&self.player_bus_name, toggle_shuffle).flatten()
                {
                    self.shuffle = shuffle;
                    coordinator::request_refresh();
                }
            }
            Message::CycleLoop => {
                if let Some(loop_status) =
                    with_player(&self.player_bus_name, cycle_loop_status).flatten()
                {
                    self.loop_status = Some(loop_status);
                    coordinator::request_refresh();
                }
            }
            Message::Seek(offset) => {
                let _ = with_player(&self.player_bus_name, |player| {
                    let _ = player.seek(offset);
                });
            }
            Message::SeekTo(seconds) => {
                let current = self.position_seconds.unwrap_or(0);
                let offset = i64::try_from(seconds.saturating_sub(current)).unwrap_or(i64::MAX);
                let offset = if seconds < current { -offset } else { offset };
                let _ = with_player(&self.player_bus_name, |player| {
                    let _ = player.seek(offset.saturating_mul(1_000_000));
                });
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        coordinator::subscription().map(Message::NowPlayingChanged)
    }

    fn view(&self) -> Element<'_, Message> {
        if !self.has_active_media() {
            return self.core.applet.autosize_window(text("")).into();
        }

        let pad = self.core.applet.suggested_padding(true);
        let transport_icon = match self.playback_state {
            PlaybackState::Playing => "media-playback-pause-symbolic",
            PlaybackState::Paused | PlaybackState::Stopped | PlaybackState::Unknown => {
                "media-playback-start-symbolic"
            }
        };

        // Compact card: player icon, cover, two small text lines, then the transport.
        const PANEL_TRANSPORT_SIZE: f32 = 22.0;
        const PANEL_TRANSPORT_ICON: u16 = 14;
        const PANEL_PLAYER_ICON: u16 = 16;
        const PANEL_ART_SIZE: f32 = 26.0;
        const PANEL_TEXT_WIDTH: f32 = 150.0;
        const PANEL_TITLE_SIZE: f32 = 11.0;
        const PANEL_ARTIST_SIZE: f32 = 9.5;
        let transport = |name: &'static str| {
            container(icon::from_name(name).size(PANEL_TRANSPORT_ICON))
                .width(Length::Fixed(PANEL_TRANSPORT_SIZE))
                .height(Length::Fixed(PANEL_TRANSPORT_SIZE))
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center)
        };
        let panel_previous: Element<'_, Message> = if self.can_go_previous {
            mouse_area(transport("media-skip-backward-symbolic"))
                .on_press(Message::PreviousTrack)
                .into()
        } else {
            transport("media-skip-backward-symbolic").into()
        };
        let panel_play: Element<'_, Message> = mouse_area(transport(transport_icon))
            .on_press(Message::TogglePlayPause)
            .into();
        let panel_next: Element<'_, Message> = if self.can_go_next {
            mouse_area(transport("media-skip-forward-symbolic"))
                .on_press(Message::NextTrack)
                .into()
        } else {
            transport("media-skip-forward-symbolic").into()
        };
        let player_icon = container(
            icon::from_name(player_icon_name(&self.player_bus_name)).size(PANEL_PLAYER_ICON),
        )
        .width(Length::Fixed(PANEL_PLAYER_ICON as f32))
        .height(Length::Fixed(PANEL_PLAYER_ICON as f32))
        .align_x(cosmic::iced::alignment::Horizontal::Center)
        .align_y(cosmic::iced::alignment::Vertical::Center);
        let panel_art: Element<'_, Message> = if let Some(path) = self.album_art_path.as_ref() {
            container(
                image(image::Handle::from_path(path.clone()))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(ContentFit::Cover)
                    .border_radius(5.0),
            )
            .width(Length::Fixed(PANEL_ART_SIZE))
            .height(Length::Fixed(PANEL_ART_SIZE))
            .clip(true)
            .into()
        } else {
            Space::new()
                .width(Length::Fixed(PANEL_ART_SIZE))
                .height(Length::Fixed(PANEL_ART_SIZE))
                .into()
        };
        let artist_line = if self.now_playing_artist.is_empty() {
            self.now_playing_album.as_str()
        } else {
            self.now_playing_artist.as_str()
        };
        let lines = Column::new()
            .spacing(0)
            .width(Length::Fixed(PANEL_TEXT_WIDTH))
            .push(
                text(self.now_playing_title.as_str())
                    .size(PANEL_TITLE_SIZE)
                    .width(Length::Fixed(PANEL_TEXT_WIDTH))
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            )
            .push(
                text(artist_line)
                    .size(PANEL_ARTIST_SIZE)
                    .width(Length::Fixed(PANEL_TEXT_WIDTH))
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                    .class(cosmic::theme::Text::Default),
            );
        let panel_content = Row::new()
            .spacing(8)
            .align_y(cosmic::iced::alignment::Vertical::Center)
            .push(player_icon)
            .push(panel_art)
            .push(lines)
            .push(
                Row::new()
                    .spacing(2)
                    .align_y(cosmic::iced::alignment::Vertical::Center)
                    .push(panel_previous)
                    .push(panel_play)
                    .push(panel_next),
            );
        let panel_card = button::custom(panel_content)
            // A single applet button provides one shared hover surface; the
            // mouse areas above capture transport clicks before this popup action.
            .padding([2, 6, 2, 6])
            .class(cosmic::theme::Button::AppletIcon)
            .on_press_with_rectangle(|offset, bounds| Message::TogglePopup {
                anchor_x: (bounds.x - offset.x).round() as i32,
                anchor_y: (bounds.y - offset.y).round() as i32,
                anchor_width: bounds.width.round() as i32,
                anchor_height: bounds.height.round() as i32,
            });
        let row_content = Row::new()
            .padding([0, pad.0])
            .align_y(cosmic::iced::alignment::Vertical::Center)
            .push(panel_card);

        // The complete panel control is one visual unit, while transport
        // regions retain their own input handling.
        self.core.applet.autosize_window(row_content).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        if !self.has_active_media() {
            return self.core.applet.popup_container(text("")).into();
        }

        let size = self.core.applet.suggested_size(true);
        let pad = self.core.applet.suggested_padding(true);
        let transport_icon = match self.playback_state {
            PlaybackState::Playing => "media-playback-pause-symbolic",
            PlaybackState::Paused | PlaybackState::Stopped | PlaybackState::Unknown => {
                "media-playback-start-symbolic"
            }
        };
        // MediaShell uses the popup's full content width for cover art.  The
        // previous 16:9 box both distorted the visual hierarchy and left a
        // large, awkward amount of unused popup width.
        const POPUP_CONTENT_WIDTH: f32 = 328.0;

        let album_widget: Element<'_, Message> = if let Some(path) = self.album_art_path.as_ref() {
            image(image::Handle::from_path(path.clone()))
                .height(Length::Fixed(POPUP_CONTENT_WIDTH))
                .width(Length::Fixed(POPUP_CONTENT_WIDTH))
                // Preserve unusually tall/wide artwork inside the square card.
                // `Cover` can enlarge it beyond the image bounds on some
                // fractional-scale renderers.
                .content_fit(ContentFit::Contain)
                .border_radius(12.0)
                .into()
        } else {
            icon::from_name("audio-x-generic-symbolic")
                .size(72)
                .icon()
                .height(Length::Fixed(POPUP_CONTENT_WIDTH))
                .width(Length::Fixed(POPUP_CONTENT_WIDTH))
                .content_fit(ContentFit::Contain)
                .into()
        };

        let previous =
            button::icon(icon::from_name("media-skip-backward-symbolic").size(size.0 + 4));
        let previous = if self.can_go_previous {
            previous.on_press(Message::PreviousTrack)
        } else {
            previous
        };
        let next = button::icon(icon::from_name("media-skip-forward-symbolic").size(size.0 + 4));
        let next = if self.can_go_next {
            next.on_press(Message::NextTrack)
        } else {
            next
        };
        let controls = Row::new()
            .spacing(pad.0.saturating_add(6))
            .align_y(cosmic::iced::alignment::Vertical::Center)
            .push(if self.can_seek {
                button::icon(icon::from_name("media-seek-backward-symbolic").size(size.0))
                    .on_press(Message::Seek(-10_000_000))
            } else {
                button::icon(icon::from_name("media-seek-backward-symbolic").size(size.0))
            })
            .push(previous)
            .push(
                button::icon(icon::from_name(transport_icon).size(size.0 + 8))
                    .class(cosmic::theme::Button::Suggested)
                    .on_press(Message::TogglePlayPause),
            )
            .push(next)
            .push(if self.can_seek {
                button::icon(icon::from_name("media-seek-forward-symbolic").size(size.0))
                    .on_press(Message::Seek(10_000_000))
            } else {
                button::icon(icon::from_name("media-seek-forward-symbolic").size(size.0))
            });

        let media_info = Column::new()
            .spacing(4)
            .align_x(cosmic::iced::Alignment::Start)
            .push(
                text(self.now_playing_title.as_str())
                    .size(size.0.saturating_add(5))
                    .width(Length::Fixed(POPUP_CONTENT_WIDTH))
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            )
            .push(
                text(self.now_playing_artist.as_str())
                    .size(size.0)
                    .width(Length::Fixed(POPUP_CONTENT_WIDTH))
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            )
            .push(
                text(self.now_playing_album.as_str())
                    .size(size.0.saturating_sub(2))
                    .width(Length::Fixed(POPUP_CONTENT_WIDTH))
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
            );

        let position = self.position_seconds.unwrap_or(0);
        let duration = self.duration_seconds.unwrap_or(0);
        let progress = format!(
            "{}  /  {}",
            format_duration(position),
            format_duration(duration)
        );
        let progress_bar = if self.can_seek && duration > 0 {
            slider(
                0.0..=duration as f32,
                position.min(duration) as f32,
                |value| Message::SeekTo(value.round() as u64),
            )
            .width(Length::Fixed(220.0))
        } else {
            slider(0.0..=1.0, 0.0, |_| Message::SeekTo(0)).width(Length::Fixed(220.0))
        };
        let progress_row = Row::new()
            .spacing(8)
            .align_y(cosmic::iced::alignment::Vertical::Center)
            .push(progress_bar)
            .push(
                text(progress)
                    .width(Length::Fixed(88.0))
                    .wrapping(Wrapping::None)
                    .align_x(cosmic::iced::alignment::Horizontal::Right),
            );

        let loop_status = self.loop_status.unwrap_or(LoopStatus::None);
        // Keep both mode controls in equal-width slots. Repeat-one adds a
        // small marker, but must not make the whole row jump sideways.
        const MODE_BUTTON_WIDTH: f32 = 30.0;
        let shuffle = button::custom(
            container(icon::from_name("media-playlist-shuffle-symbolic").size(size.0))
                .width(Length::Fixed(MODE_BUTTON_WIDTH))
                .height(Length::Fixed(MODE_BUTTON_WIDTH))
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center),
        );
        let shuffle = if self.can_shuffle {
            shuffle.on_press(Message::ToggleShuffle)
        } else {
            shuffle
        };
        let shuffle = if self.shuffle {
            shuffle.class(cosmic::theme::Button::Suggested)
        } else {
            shuffle
        };
        let loop_content = cosmic::iced::widget::stack(vec![
            container(icon::from_name("media-playlist-repeat-symbolic").size(size.0))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center)
                .into(),
            container(
                text(if loop_status == LoopStatus::Track {
                    "1"
                } else {
                    ""
                })
                .size(size.0.saturating_sub(4)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(cosmic::iced::alignment::Horizontal::Right)
            .align_y(cosmic::iced::alignment::Vertical::Bottom)
            .into(),
        ])
        .width(Length::Fixed(MODE_BUTTON_WIDTH))
        .height(Length::Fixed(MODE_BUTTON_WIDTH));
        let loop_button = button::custom(loop_content);
        let loop_button = if self.can_loop {
            loop_button.on_press(Message::CycleLoop)
        } else {
            loop_button
        };
        let loop_button = if loop_status != LoopStatus::None {
            loop_button.class(cosmic::theme::Button::Suggested)
        } else {
            loop_button
        };
        let mode_controls = Row::new().spacing(8).push(shuffle).push(loop_button);

        let source_picker =
            self.sources
                .iter()
                .fold(Row::new().spacing(4), |row, (bus_name, name)| {
                    let label = if bus_name == &self.player_bus_name {
                        format!("• {name}")
                    } else {
                        name.clone()
                    };
                    row.push(
                        button::standard(label)
                            .class(if bus_name == &self.player_bus_name {
                                cosmic::theme::Button::Suggested
                            } else {
                                cosmic::theme::Button::Standard
                            })
                            .on_press(Message::SelectSource(bus_name.clone())),
                    )
                });

        let content_list = Column::new()
            .padding(16)
            .spacing(12)
            .align_x(cosmic::iced::Alignment::Center)
            // A fixed header prevents the cover image from being laid out over
            // the source controls on fractional display scales.
            .push(
                container(source_picker)
                    .height(Length::Fixed(36.0))
                    .align_y(cosmic::iced::alignment::Vertical::Center),
            )
            .push(album_widget)
            .push(media_info)
            .push(progress_row)
            .push(controls)
            .push(mode_controls);

        // Match COSMIC's own applets: the applet popup owns the themed card
        // surface, while our content is only its child. This keeps its
        // background, border, radius and text colours under COSMIC control.
        self.core
            .applet
            .popup_container(container(content_list))
            .into()
    }
}

impl Window {
    fn apply_now_playing(&mut self, data: NowPlayingData) {
        self.now_playing_text = data.text;
        self.now_playing_title = data.title;
        self.now_playing_artist = data.artist;
        self.now_playing_album = data.album;
        self.player_bus_name = data.player_bus_name;
        self.sources = data.sources;
        self.duration_seconds = data.duration_seconds;
        self.position_seconds = data.position_seconds;
        self.can_seek = data.capabilities.seek;
        self.can_go_previous = data.capabilities.previous;
        self.can_go_next = data.capabilities.next;
        self.can_shuffle = data.capabilities.shuffle;
        self.shuffle = data.shuffle;
        self.can_loop = data.capabilities.loop_mode;
        self.loop_status = Some(data.loop_status);
        self.playback_state = data.state;
        self.album_art_path = data.album_art_path;
        self.album_art_dimensions = data.album_art_dimensions;
        self.has_active_media = data.has_active_media;
    }
}

fn format_duration(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Window {
    fn has_active_media(&self) -> bool {
        self.has_active_media
    }
}
