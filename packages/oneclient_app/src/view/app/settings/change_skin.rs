use freya::prelude::*;
use freya::query::MutationStateData;
use oneclient_auth::AccountKind;
use oneclient_mc::SkinVariant;
use uuid::Uuid;

use super::accounts::mutation_err_text;
use crate::components::{
    Button, Icon, IconType, OverlayPopup, PlayerModel, SwitchSide, pill_switch,
};
use crate::hooks::{ChangeSkinKeys, UseChangeSkin, use_change_skin};
use crate::i18n::t;
use crate::theme::{colors, rounded, text_size};
use crate::ui::border_all_color;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

const MODEL_WIDTH_PX: f32 = 120.;
const MODEL_HEIGHT_PX: f32 = 200.;

#[derive(PartialEq)]
pub struct ChangeSkinPopup {
    pub id: Uuid,
    pub username: String,
    pub kind: AccountKind,
    pub on_close: EventHandler<()>,
}

impl Component for ChangeSkinPopup {
    fn render_key(&self) -> DiffKey {
        DiffKey::from(&self.id)
    }

    fn render(&self) -> impl IntoElement {
        let change_skin: UseChangeSkin = use_change_skin();
        let mut picked = use_state::<Option<(String, Vec<u8>)>>(|| None);
        let mut pick_error = use_state(String::new);
        let mut slim = use_state(|| false);

        let is_offline = self.kind == AccountKind::Offline;
        let id = self.id;
        let on_close = self.on_close.clone();
        let on_close_ok = self.on_close.clone();
        let on_close_btn = self.on_close.clone();

        use_side_effect(move || match &*change_skin.read().state() {
            MutationStateData::Settled { res: Ok(_), .. } => {
                on_close_ok.call(());
            }
            _ => {}
        });

        let error = mutation_err_text(&change_skin).or_else(|| {
            let pick_error = pick_error.read().clone();
            (!pick_error.is_empty()).then_some(pick_error)
        });

        let has_file = picked.read().is_some();

        let variant = if *slim.read() {
            SkinVariant::Slim
        } else {
            SkinVariant::Classic
        };

        let on_pick = move |_| {
            spawn(async move {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_title(t("oneclient.settings.accounts.skin.picker.title"))
                    .add_filter("PNG", &["png"])
                    .pick_file()
                    .await
                else {
                    return;
                };
                let png = handle.read().await;
                if !png.starts_with(PNG_MAGIC) {
                    pick_error.set(t("oneclient.settings.accounts.skin.invalid_png").to_string());
                    return;
                }
                pick_error.set(String::new());
                picked.set(Some((handle.file_name(), png)));
            });
        };

        let on_confirm = move |_| {
            let Some((_, png)) = picked.peek().clone() else {
                return;
            };
            change_skin.mutate(ChangeSkinKeys {
                id,
                png,
                variant: variant.clone(),
            });
        };

        let classic_side = SwitchSide {
            label: t("oneclient.settings.accounts.skin.classic"),
            active: !*slim.read(),
            accent: true,
            on_press: EventHandler::new(move |()| slim.set(false)),
        };
        let slim_side = SwitchSide {
            label: t("oneclient.settings.accounts.skin.slim"),
            active: *slim.read(),
            accent: true,
            on_press: EventHandler::new(move |()| slim.set(true)),
        };

        OverlayPopup::new()
            .on_close(move |()| on_close.call(()))
            .child(
                rect()
                    .width(Size::window_percent(100.))
                    .height(Size::window_percent(100.))
                    .center()
                    .child(
                        rect()
                            .vertical()
                            .width(Size::px(380.))
                            .max_width(Size::window_percent(90.))
                            .spacing(16.)
                            .padding(Gaps::new_all(20.))
                            .corner_radius(rounded(16.))
                            .background(colors::page_elevated())
                            .border(border_all_color(1., colors::component_border()))
                            .child(
                                label()
                                    .text(t("oneclient.settings.accounts.skin.title"))
                                    .font_size(text_size(18.))
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .color(colors::fg_primary()),
                            )
                            .child(
                                rect()
                                    .horizontal()
                                    .width(Size::fill())
                                    .content(Content::Flex)
                                    .spacing(16.)
                                    .cross_align(Alignment::Center)
                                    .child(
                                        rect()
                                            .width(Size::px(MODEL_WIDTH_PX))
                                            .height(Size::px(MODEL_HEIGHT_PX))
                                            .center()
                                            .overflow(Overflow::Clip)
                                            .corner_radius(rounded(12.))
                                            .background(colors::component_bg())
                                            .border(border_all_color(
                                                1.,
                                                colors::component_border(),
                                            ))
                                            .child(
                                                PlayerModel::new(self.id.to_string())
                                                    .yaw(-0.5)
                                                    .width(Size::fill())
                                                    .height(Size::fill()),
                                            ),
                                    )
                                    .child(
                                        rect()
                                            .vertical()
                                            .width(Size::flex(1.0))
                                            .spacing(8.)
                                            .child(
                                                label()
                                                    .text(if is_offline {
                                                        t("oneclient.settings.accounts.skin.offline_note")
                                                    } else {
                                                        t("oneclient.settings.accounts.skin.microsoft_note")
                                                    })
                                                    .font_size(text_size(11.))
                                                    .color(colors::fg_secondary()),
                                            )
                                            .child(pill_switch([classic_side, slim_side])),
                                    ),
                            )
                            .child(
                                rect()
                                    .horizontal()
                                    .width(Size::fill())
                                    .cross_align(Alignment::Center)
                                    .content(Content::Flex)
                                    .spacing(8.)
                                    .child(
                                        Button::new()
                                            .secondary()
                                            .on_press(on_pick)
                                            .child(Icon::new(IconType::File02).size(16.))
                                            .text(t("oneclient.settings.accounts.skin.pick")),
                                    )
                                    .child(
                                        label()
                                            .text(
                                                picked.read().as_ref().map_or_else(
                                                    || "-".to_string(),
                                                    |(name, _)| name.clone(),
                                                ),
                                            )
                                            .font_size(text_size(11.))
                                            .max_lines(1)
                                            .text_overflow(TextOverflow::Ellipsis)
                                            .color(colors::fg_secondary()),
                                    ),
                            )
                            .map(error, |el, msg| {
                                el.child(hint_line(IconType::AlertTriangle, msg, colors::danger()))
                            })
                            .child(
                                rect()
                                    .horizontal()
                                    .width(Size::fill())
                                    .main_align(Alignment::End)
                                    .spacing(8.)
                                    .child(
                                        Button::new()
                                            .ghost()
                                            .on_press(move |_| on_close_btn.call(()))
                                            .text(t("oneclient.common.cancel")),
                                    )
                                    .child(
                                        Button::new()
                                            .primary()
                                            .enabled(has_file)
                                            .on_press(on_confirm)
                                            .child(Icon::new(IconType::CheckCircle).size(16.))
                                            .text(t("oneclient.settings.accounts.skin.confirm")),
                                    ),
                            ),
                    ),
            )
            .into_element()
    }
}

fn hint_line(icon: IconType, text: String, color: Color) -> impl IntoElement {
    rect()
        .horizontal()
        .width(Size::fill())
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(Icon::new(icon).size(13.).color(color))
        .child(
            label()
                .text(text)
                .font_size(text_size(12.))
                .width(Size::flex(1.0))
                .color(color),
        )
        .into_element()
}
