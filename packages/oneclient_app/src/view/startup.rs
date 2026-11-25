use freya::animation::{AnimNum, Ease, Function, OnCreation, OnFinish, use_animation};
use freya::{prelude::*, router::*};

use crate::AppAssets;
use crate::Route;
use crate::components::{Button, progress_track};
use crate::hooks::use_dispatch;
use crate::hooks::{
    terms_document, terms_is_loading, use_launcher, use_notifications_snapshot,
    use_settings_snapshot, use_splash, use_terms,
};
use crate::i18n::{t, tf};
use crate::theme::{colors, rounded, text_size};
use crate::ui::border_all_color;

/// Startup is a sequence of boolean gates, not measurable work, so the bar steps
/// once per gate the launcher clears instead of tracking bytes
const STARTUP_STEPS: u8 = 3;

#[derive(PartialEq)]
struct RecoveryActions {
    snapshots: usize,
}

impl Component for RecoveryActions {
    fn render(&self) -> impl IntoElement {
        let actions = use_dispatch();
        let snapshots = self.snapshots;

        let restore = actions.clone();

        let mut row = rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(8.)
            .child(
                Button::new()
                    .secondary()
                    .small()
                    .on_press(|_| crate::recovery::open_data_folder())
                    .text(t("oneclient.startup.recovery.open_data_folder")),
            );

        if snapshots > 0 {
            row = row.child(
                Button::new()
                    .primary()
                    .small()
                    .on_press(move |_| crate::recovery::restore_latest(&restore))
                    .text(t("oneclient.startup.recovery.restore_backup")),
            );
        }

        row.child(
            Button::new()
                .danger()
                .small()
                .on_press(move |_| crate::recovery::reset(&actions))
                .text(t("oneclient.startup.recovery.reset_database")),
        )
    }
}

#[derive(PartialEq)]
pub struct Startup;

impl Component for Startup {
    fn render(&self) -> impl IntoElement {
        let launcher = use_launcher();
        let settings = use_settings_snapshot();
        let notifications = use_notifications_snapshot();
        let terms = use_terms();
        let splash = use_splash();

        let intro = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            AnimNum::new(0., 1.)
                .time(900)
                .ease(Ease::InOut)
                .function(Function::Cubic)
        });

        let pulse = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            conf.on_finish(OnFinish::reverse());
            AnimNum::new(0.88, 1.0)
                .time(2400)
                .ease(Ease::InOut)
                .function(Function::Sine)
        });

        // vinlauncher splash: the V settles in slowly, then a ring keeps pinging
        // around it — slow and calm so the user can take it in
        let zoom = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            AnimNum::new(0.72, 1.0)
                .time(1600)
                .ease(Ease::InOut)
                .function(Function::Sine)
        });

        let ping = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            conf.on_finish(OnFinish::restart());
            AnimNum::new(0.0, 1.0)
                .time(2600)
                .ease(Ease::Out)
                .function(Function::Sine)
        });

        let logo = use_memo(|| AppAssets::get_bytes("logo.svg").unwrap_or_default());
        let mark = use_memo(|| AppAssets::get_bytes("icons/icon-logo.svg").unwrap_or_default());

        if launcher.needs_location {
            let _ = RouterContext::get().replace(Route::OnboardingWelcome {});
            return rect().into_element();
        }

        if launcher.ready && !terms_is_loading(&terms) {
            let document = terms_document(&terms);
            let required_terms = document.as_ref().map(|doc| doc.version).unwrap_or(1);
            let required_privacy = document
                .as_ref()
                .map(|doc| doc.privacy_version())
                .unwrap_or(1);

            let stale = settings.settings.accepted_tos_version < required_terms
                || settings.settings.accepted_privacy_version < required_privacy;

            let destination = if !settings.settings.seen_onboarding {
                Route::OnboardingWelcome {}
            } else if settings.settings.declined_tos {
                Route::Home {}
            } else if stale {
                Route::OnboardingTerms {}
            } else {
                Route::Home {}
            };

            let heading_to_onboarding = matches!(destination, Route::OnboardingWelcome { .. });
            let heading_to_terms = matches!(destination, Route::OnboardingTerms { .. });
            if !heading_to_onboarding || !launcher.fetching {
                // Raise the splash curtain so the hard route swap is hidden until Home settles onboarding animates itself
                if !heading_to_onboarding && !heading_to_terms {
                    let mut active = splash.active;
                    let mut home_ready = splash.home_ready;
                    home_ready.set(false);
                    active.set(true);
                }
                let _ = RouterContext::get().replace(destination);
                return rect().into_element();
            }
        }

        let active = notifications.inbox.iter().find(|entry| entry.is_loading);

        let (message, detail, step): (String, Option<String>, u8) =
            if let Some(err) = launcher.error.as_deref() {
                let step = if launcher.ready { 2 } else { 1 };
                (
                    t("oneclient.startup.error").to_string(),
                    Some(err.to_string()),
                    step,
                )
            } else if let Some(entry) = active {
                let detail = entry
                    .tasks
                    .iter()
                    .find(|t| t.total == 0 || t.current < t.total)
                    .map(|t| t.label.clone())
                    .or_else(|| entry.progress.map(|(c, t)| format!("{c} / {t}")));
                (entry.title.clone(), detail, 3)
            } else if launcher.ready {
                (t("oneclient.startup.fetching").to_string(), None, 2)
            } else {
                (t("oneclient.startup.starting").to_string(), None, 1)
            };

        let is_error = launcher.error.is_some();

        let appear = intro.get().value();
        let rise = (1.0 - appear) * 10.;
        let breath = pulse.read().value();
        let zoom = zoom.read().value().clamp(0.65, 1.0);
        let ping_t = ping.read().value();

        // expanding radar-ping ring behind the V mark
        let ring = rect()
            .width(Size::px(96. + 64. * ping_t))
            .height(Size::px(96. + 64. * ping_t))
            .corner_radius(CornerRadius::new_all((48. + 32. * ping_t).round() as f32))
            .border(border_all_color(
                2.,
                colors::brand().with_a(((1.0 - ping_t) * 52.) as u8),
            ))
            .center()
            .opacity(1.0 - ping_t);

        let brand_mark = rect()
            .center()
            .width(Size::px(120.))
            .height(Size::px(120.))
            .child(ring)
            .child(
                SvgViewer::new(("icons/icon-logo.svg", mark.read().cloned()))
                    .show_loader(false)
                    .width(Size::px(64.))
                    .height(Size::px(64.))
                    .color(colors::brand()),
            )
            .scale(zoom);

        let dot_color = if is_error {
            colors::danger()
        } else {
            colors::brand()
        };

        let filled = f32::from(step) / f32::from(STARTUP_STEPS + 1) * 100.;

        let progress = rect().width(Size::px(288.)).child(progress_track(
            filled,
            4.,
            dot_color,
            colors::fg_primary().with_a(30),
        ));

        let loading_row = rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(9.)
            .child(
                rect()
                    .width(Size::px(8.))
                    .height(Size::px(8.))
                    .corner_radius(rounded(4.))
                    .background(dot_color),
            )
            .child(
                label()
                    .text(message)
                    .font_size(text_size(14.))
                    .color(colors::fg_primary().with_a(180)),
            );

        let mut content = rect()
            .center()
            .vertical()
            .spacing(14.)
            .child(brand_mark)
            .child(
                SvgViewer::new(("logo.svg", logo.read().cloned()))
                    .show_loader(false)
                    .width(Size::px(260.))
                    .height(Size::px(56.))
                    .color(colors::fg_primary()),
            )
            .child(
                label()
                    .text(t("oneclient.startup.tagline"))
                    .font_size(text_size(15.))
                    .color(colors::fg_primary().with_a(140)),
            )
            .child(rect().height(Size::px(14.)))
            .child(loading_row)
            .child(progress);

        if let Some(detail) = detail {
            content = content.child(
                label()
                    .text(detail)
                    .font_size(text_size(12.))
                    .max_lines(if is_error { 4 } else { 1 })
                    .color(colors::fg_secondary().with_a(180)),
            );
        }

        if is_error {
            content = content
                .child(rect().height(Size::px(6.)))
                .child(RecoveryActions {
                    snapshots: launcher.snapshots,
                })
                .child(
                    label()
                        .text(tf("oneclient.startup.data_safe", &[&launcher.data_dir]))
                        .font_size(text_size(11.))
                        .max_lines(2)
                        .color(colors::fg_secondary().with_a(130)),
                );
        }

        let content = content
            .position(Position::new_absolute())
            .width(Size::fill())
            .height(Size::fill())
            .center()
            .opacity(appear * breath)
            .margin(Gaps::new(rise, 0., 0., 0.));

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .background(colors::page())
            .window_drag()
            .child(content)
            .into_element()
    }
}
