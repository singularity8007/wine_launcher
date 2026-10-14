//! The window, built with iced 0.14 (Elm-style: state, messages, update, view).
//!
//! It draws what `Backend::view()` says and forwards button clicks to
//! `Backend`. It never touches downloads, threads or the game, so swapping
//! this file for a different one changes nothing else.

use crate::backend::{Backend, Notice, View};
use iced::time::{self, Instant, milliseconds};
use iced::widget::{
    Column, button, center, column, container, mouse_area, opaque, progress_bar, row, space,
    stack, text,
};
use iced::{
    Background, Center, Color, Degrees, Element, Fill, Font, Length, Shadow, Size,
    Subscription, Theme, Vector, border, color, font, gradient,
};

// ---------- Colours: near-black, one white accent ----------

const INK: Color = color!(0xf2f2f5); // main text and the PLAY button
const DIM: Color = color!(0x8a8a94); // small labels
const LINE: Color = color!(0x25252b); // hairline borders
const LINE_HOVER: Color = color!(0x6a6a76);
const SURFACE: Color = color!(0x0e0e11); // cards and the popup
const GOOD: Color = color!(0x9fd8b8); // good-news text
const WARN: Color = color!(0xf28b8b); // error popup title

pub fn run(backend: Backend) -> iced::Result {
    iced::application(move || App::new(backend.clone()), App::update, App::view)
        .title("wine_launcher")
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(940.0, 600.0))
        .centered()
        .run()
}

struct App {
    backend: Backend,
    theme: Theme,
}
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Message {
    Launch,
    Install,
    CheckUpdates,
    DismissNotice,
    Tick(Instant),
}

impl App {
    fn new(backend: Backend) -> Self {
        let palette = iced::theme::Palette {
            background: color!(0x050506),
            text: INK,
            primary: INK,
            success: GOOD,
            warning: color!(0xfbbf24),
            danger: WARN,
        };
        Self {
            backend,
            theme: Theme::custom("wine_launcher", palette),
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Launch => self.backend.launch(),
            Message::Install => self.backend.install_game(),
            Message::CheckUpdates => self.backend.check_updates(),
            Message::DismissNotice => self.backend.dismiss(),
            // Nothing to do with the time itself: any message makes iced redraw,
            // which re-reads the backend. The backend just saves play time.
            Message::Tick(_) => self.backend.tick(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let view = self.backend.view();

        let page = container(
            column![
                header(&view),
                space::vertical(),
                status(&view),
                actions(view.busy),
            ]
            .spacing(24),
        )
        .padding(32)
        .width(Fill)
        .height(Fill)
        .style(page_style);

        // Errors pop up over the page; the page is still there underneath.
        match view.notice.filter(|notice| notice.is_error) {
            Some(notice) => modal(page.into(), popup(&notice)),
            None => page.into(),
        }
    }

    /// A steady tick so the progress bar and play time stay current. Slower
    /// while nothing is happening.
    fn subscription(&self) -> Subscription<Message> {
        let every = if self.backend.is_busy() {
            milliseconds(250)
        } else {
            milliseconds(1000)
        };
        time::every(every).map(Message::Tick)
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}

// ---------- Layout ----------

fn header<'a>(view: &View) -> Element<'a, Message> {
    let title = column![
        text("ZENLESS ZONE ZERO").size(28).font(bold()),
        text("wine_launcher").size(13).color(DIM),
    ]
    .spacing(4);

    let stats = container(
        row![
            stat("PLAY TIME", view.play_time.clone()),
            stat("LAST PLAYED", view.last_played.clone()),
        ]
        .spacing(32),
    )
    .padding([12, 22])
    .style(card_style);

    row![title, space::horizontal(), stats].align_y(Center).into()
}

fn stat<'a>(label: &'static str, value: String) -> Column<'a, Message> {
    column![
        text(label).size(11).font(bold()).color(DIM),
        text(value).size(17).font(bold()),
    ]
    .spacing(3)
}

/// The progress bar while a job runs, or the last good-news message.
fn status<'a>(view: &View) -> Element<'a, Message> {
    if view.busy {
        let mut body = Column::new().spacing(10).push(text(view.label.clone()).size(14).color(DIM));
        if let Some(fraction) = view.fraction {
            body = body.push(progress_bar(0.0..=1.0, fraction).girth(4.0).style(bar_style));
        }
        return body.into();
    }

    match &view.notice {
        Some(notice) if !notice.is_error => {
            text(notice.text.clone()).size(14).color(GOOD).into()
        }
        _ => space().into(),
    }
}

fn actions<'a>(busy: bool) -> Element<'a, Message> {
    // `None` greys a button out and makes it ignore clicks.
    let when_free = |message: Message| (!busy).then_some(message);

    let play = button(text("PLAY").size(20).font(bold()).width(Fill).center())
        .on_press_maybe(when_free(Message::Launch))
        .width(Length::Fixed(240.0))
        .padding(15)
        .style(play_button);

    let install = button(text("Install game").size(14).center())
        .on_press_maybe(when_free(Message::Install))
        .padding([14, 24])
        .style(secondary_button);

    let check = button(text("Check for updates").size(14).center())
        .on_press_maybe(when_free(Message::CheckUpdates))
        .padding([14, 24])
        .style(secondary_button);

    row![play, install, check].spacing(12).align_y(Center).into()
}

/// The error popup: a small card with a small x in the corner.
fn popup<'a>(notice: &Notice) -> Element<'a, Message> {
    let close = button(text("×").size(18).width(Fill).center())
        .on_press(Message::DismissNotice)
        .width(Length::Fixed(26.0))
        .height(Length::Fixed(26.0))
        .padding(0)
        .style(close_button);

    let mut body = Column::new()
        .spacing(12)
        .push(
            row![
                text("Heads up").size(17).font(bold()).color(WARN),
                space::horizontal(),
                close
            ]
            .align_y(Center),
        )
        .push(text(notice.text.clone()).size(14));

    if let Some(details) = &notice.details {
        body = body.push(
            text(format!("Technical details: {details}"))
                .size(11)
                .color(DIM),
        );
    }

    container(body)
        .width(Length::Fixed(400.0))
        .padding(18)
        .style(popup_style)
        .into()
}

/// Dim the page and show `content` on top. Clicking outside the card closes it.
fn modal<'a>(base: Element<'a, Message>, content: Element<'a, Message>) -> Element<'a, Message> {
    stack![
        base,
        opaque(
            mouse_area(center(opaque(content)).style(|_theme| container::Style {
                background: Some(
                    Color {
                        a: 0.75,
                        ..Color::BLACK
                    }
                    .into()
                ),
                ..container::Style::default()
            }))
            .on_press(Message::DismissNotice)
        )
    ]
    .into()
}

// ---------- Look ----------

fn bold() -> Font {
    Font {
        weight: font::Weight::Bold,
        ..Font::DEFAULT
    }
}

/// Almost-black, with a barely-there lift at the top.
fn page_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::from(
            gradient::Linear::new(Degrees(160.0))
                .add_stop(0.0, color!(0x0d0d11))
                .add_stop(1.0, color!(0x040405)),
        )),
        ..container::Style::default()
    }
}

fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border::rounded(12.0).color(LINE).width(1.0),
        ..container::Style::default()
    }
}

fn popup_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border::rounded(14.0).color(color!(0x34343c)).width(1.0),
        shadow: Shadow {
            color: Color {
                a: 0.8,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 10.0),
            blur_radius: 30.0,
        },
        ..container::Style::default()
    }
}

/// A thin white line that fills as the job progresses.
fn bar_style(_theme: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: color!(0x1a1a1f).into(),
        bar: INK.into(),
        border: border::rounded(2.0),
    }
}

/// The big solid white PLAY button.
fn play_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Active => (INK, Color::BLACK),
        button::Status::Hovered => (Color::WHITE, Color::BLACK),
        button::Status::Pressed => (color!(0xc4c4cc), Color::BLACK),
        button::Status::Disabled => (color!(0x1a1a1f), color!(0x55555e)),
    };
    let glow = matches!(status, button::Status::Hovered);

    button::Style {
        background: Some(background.into()),
        text_color,
        border: border::rounded(10.0),
        shadow: if glow {
            Shadow {
                color: Color { a: 0.2, ..Color::WHITE },
                offset: Vector::new(0.0, 0.0),
                blur_radius: 24.0,
            }
        } else {
            Shadow::default()
        },
        snap: false,
    }
}

/// The quiet outlined buttons next to PLAY.
fn secondary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, outline, text_color) = match status {
        button::Status::Active => (Color::TRANSPARENT, LINE, color!(0xd8d8de)),
        button::Status::Hovered => (color!(0x121216), LINE_HOVER, Color::WHITE),
        button::Status::Pressed => (color!(0x0a0a0c), LINE_HOVER, color!(0xd8d8de)),
        button::Status::Disabled => (Color::TRANSPARENT, color!(0x18181c), color!(0x4a4a52)),
    };

    button::Style {
        background: Some(background.into()),
        text_color,
        border: border::rounded(10.0).color(outline).width(1.0),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// The small x on the error popup.
fn close_button(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0x26262d),
        _ => Color::TRANSPARENT,
    };

    button::Style {
        background: Some(background.into()),
        text_color: color!(0xb0b0ba),
        border: border::rounded(8.0),
        shadow: Shadow::default(),
        snap: false,
    }
}