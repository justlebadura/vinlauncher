//! Página de Amigos — sistema P2P serverless.
//!
//! Flujo: cada usuario genera un código de invitación, lo comparte por
//! cualquier medio, pega el código del amigo y ya pueden transferirse
//! instancias (.mrpack) directamente entre sí.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use freya::prelude::*;

use oneclient_friends::{FriendsEvent, FriendsService};

use crate::components::{Button, TextInput};
use crate::i18n::{t, tf};
use crate::launcher;
use crate::theme::{colors, text_size};
use crate::ui::centered_note;

/// Servicio global de amigos; se inicializa con la cuenta activa.
static FRIENDS_SERVICE: OnceLock<Arc<FriendsService>> = OnceLock::new();

type Entry = oneclient_friends::identity::FriendEntry;

#[derive(PartialEq)]
pub struct Friends;

impl Component for Friends {
    fn render(&self) -> impl IntoElement {
        let mut service_state = use_state(|| Option::<Arc<FriendsService>>::None);
        let mut my_code = use_state(|| None::<String>);
        let mut invite_error = use_state(|| None::<String>);
        let mut friends_list = use_state(Vec::<Entry>::new);
        let mut new_code_input = use_state(String::new);
        let mut status_log = use_state(Vec::<String>::new);

        // Inicializa el servicio una vez (usa la cuenta de Minecraft activa).
        spawn(async move {
            if FRIENDS_SERVICE.get().is_some() {
                return;
            }
            match init_service().await {
                Ok(svc) => {
                    let code = svc.create_invite().await.ok();
                    let _ = FRIENDS_SERVICE.set(svc.clone());
                    service_state.set(Some(svc.clone()));
                    my_code.set(code);
                    friends_list.set(svc.store.list().await);
                    status_log.write().push(t("oneclient.friends.ready").to_string());
                }
                Err(e) => {
                    status_log
                        .write()
                        .push(tf("oneclient.friends.init_failed", &[&e]));
                }
            }
        });

        // Bomba de eventos del servicio -> log de estado.
        spawn(async move {
            loop {
                let svc = loop {
                    if let Some(svc) = FRIENDS_SERVICE.get() {
                        break svc.clone();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                };
                while let Some(event) = svc.next_event().await {
                    let line = match event {
                        FriendsEvent::FileReceived(from, dest, name) => tf(
                            "oneclient.friends.event.received",
                            &[&from, &name, &dest.display().to_string()],
                        ),
                        FriendsEvent::FileSent(to, name) => {
                            tf("oneclient.friends.event.sent", &[&name, &to])
                        }
                        FriendsEvent::FileQueued(to, name) => {
                            tf("oneclient.friends.event.queued", &[&to, &name])
                        }
                        FriendsEvent::PeerConnected(user, _) => {
                            tf("oneclient.friends.event.connected", &[&user])
                        }
                        FriendsEvent::Error(e) => format!("⚠ {e}"),
                    };
                    status_log.write().push(line);
                    friends_list.set(svc.store.list().await);
                }
            }
        });

        let svc = service_state.read().clone();

        rect()
            .vertical()
            .width(Size::fill())
            .height(Size::fill())
            .overflow(Overflow::Clip)
            .child(
                rect()
                    .vertical()
                    .width(Size::fill())
                    .padding(Gaps::new(0., 40., 40., 40.))
                    .spacing(24.)
                    .child(header())
                    .child(match &svc {
                        None => centered_note(t("oneclient.friends.starting")),
                        Some(svc) => rect()
                            .vertical()
                            .width(Size::fill())
                            .spacing(24.)
                            .child(my_invite_card(my_code, invite_error))
                            .child(add_friend_card(
                                new_code_input,
                                invite_error,
                                svc.clone(),
                            ))
                            .child(friends_section(
                                svc.clone(),
                                friends_list.read().clone(),
                                status_log,
                            ))
                            .into_element(),
                    }),
            )
    }
}

fn header() -> Element {
    rect()
        .vertical()
        .spacing(2.)
        .child(
            label()
                .text(t("oneclient.friends.title"))
                .font_size(text_size(32.))
                .font_weight(FontWeight::BOLD)
                .color(colors::fg_primary()),
        )
        .child(
            label()
                .text(t("oneclient.friends.description"))
                .font_size(text_size(14.))
                .color(colors::fg_secondary()),
        )
        .into_element()
}

/// Tarjeta con MI código de invitación.
fn my_invite_card(my_code: State<Option<String>>, mut err: State<Option<String>>) -> Element {
    rect()
        .vertical()
        .spacing(8.)
        .padding(Gaps::new_all(16.))
        .corner_radius(12.)
        .background(colors::component_bg())
        .child(
            label()
                .text(t("oneclient.friends.my_code.title"))
                .font_size(text_size(16.))
                .font_weight(FontWeight::BOLD)
                .color(colors::fg_primary()),
        )
        .child(
            label()
                .text(t("oneclient.friends.my_code.description"))
                .font_size(text_size(12.))
                .color(colors::fg_secondary()),
        )
        .child(match my_code.read().clone() {
            Some(code) => rect()
                .horizontal()
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .text(format!("{}…", &code[..code.len().min(56)]))
                        .font_size(text_size(12.))
                        .color(colors::fg_secondary())
                        .max_lines(1),
                )
                .child(
                    Button::new()
                        .secondary()
                        .on_press(move |_| {
                            let code = my_code.read().clone().unwrap_or_default();
                            if let Err(e) = freya::text_edit::Clipboard::set(code) {
                                err.set(Some(tf("oneclient.friends.my_code.copy_failed", &[&format!("{e:?}")])));
                            }
                        })
                        .text(t("oneclient.friends.my_code.copy")),
                )
                .into_element(),
            None => label()
                .text(t("oneclient.friends.my_code.generating"))
                .font_size(text_size(12.))
                .color(colors::fg_secondary())
                .into_element(),
        })
        .into_element()
}

/// Tarjeta para pegar el código del amigo.
fn add_friend_card(
    input: State<String>,
    mut err: State<Option<String>>,
    svc: Arc<FriendsService>,
) -> Element {
    rect()
        .vertical()
        .spacing(8.)
        .padding(Gaps::new_all(16.))
        .corner_radius(12.)
        .background(colors::component_bg())
        .child(
            label()
                .text(t("oneclient.friends.add.title"))
                .font_size(text_size(16.))
                .font_weight(FontWeight::BOLD)
                .color(colors::fg_primary()),
        )
        .child(
            rect()
                .horizontal()
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(TextInput::new(input.clone()).width(Size::fill()))
                .child(
                    Button::new().primary().on_press(move |_| {
                        let svc = Arc::clone(&svc);
                        let code = input.read().clone();
                        spawn(async move {
                            match svc.add_from_invite(&code).await {
                                Ok(_) => {}
                                Err(e) => {
                                    err.set(Some(tf("oneclient.friends.add.invalid", &[&e])));
                                }
                            }
                        });
                    })
                    .text(t("oneclient.friends.add.button")),
                ),
        )
        .child(match err.read().clone() {
            Some(e) => label()
                .text(e)
                .font_size(text_size(12.))
                .color(colors::danger())
                .into_element(),
            None => label().text("").into_element(),
        })
        .into_element()
}

/// Lista de amigos + enviar instancia + log de estado.
fn friends_section(
    svc: Arc<FriendsService>,
    friends: Vec<Entry>,
    status_log: State<Vec<String>>,
) -> Element {
    rect()
        .vertical()
        .spacing(8.)
        .child(
            label()
                .text(tf("oneclient.friends.list.title", &[&friends.len().to_string()]))
                .font_size(text_size(16.))
                .font_weight(FontWeight::BOLD)
                .color(colors::fg_primary()),
        )
        .children(if friends.is_empty() {
            vec![centered_note(t("oneclient.friends.empty"))]
        } else {
            friends
                .iter()
                .map(|friend| friend_row(svc.clone(), friend.clone(), status_log.clone()))
                .collect::<Vec<_>>()
        })
        .child(
            rect()
                .vertical()
                .spacing(4.)
                .margin(Gaps::new(16., 0., 0., 0.))
                .child(
                    label()
                        .text(t("oneclient.friends.activity"))
                        .font_size(text_size(14.))
                        .font_weight(FontWeight::BOLD)
                        .color(colors::fg_secondary()),
                )
                .children(
                    status_log
                        .read()
                        .clone()
                        .into_iter()
                        .rev()
                        .take(8)
                        .map(|line| {
                            label()
                                .text(line)
                                .font_size(text_size(12.))
                                .color(colors::fg_secondary())
                        })
                        .collect::<Vec<_>>(),
                ),
        )
        .into_element()
}

fn friend_row(
    svc: Arc<FriendsService>,
    friend: Entry,
    status_log: State<Vec<String>>,
) -> Element {
    let name = friend
        .label
        .clone()
        .unwrap_or_else(|| friend.identity.username.clone());
    let user_id = friend.identity.user_id.clone();
    let last_seen = if friend.last_seen.is_some() {
        t("oneclient.friends.seen_online").to_string()
    } else {
        t("oneclient.friends.never_connected").to_string()
    };

    let svc_send = Arc::clone(&svc);
    let name_send = name.clone();
    let user_id_send = user_id.clone();
    let status_send = status_log.clone();
    let svc_remove = Arc::clone(&svc);
    let user_id_remove = user_id.clone();
    let status_remove = status_log.clone();

    rect()
        .horizontal()
        .spacing(12.)
        .cross_align(Alignment::Center)
        .padding(Gaps::new(10., 14., 10., 14.))
        .corner_radius(10.)
        .background(colors::component_bg())
        .width(Size::fill())
        .child(
            rect()
                .vertical()
                .width(Size::flex(1.))
                .child(
                    label()
                        .text(name)
                        .font_size(text_size(14.))
                        .color(colors::fg_primary()),
                )
                .child(
                    label()
                        .text(format!("{} · {last_seen}", friend.identity.user_id))
                        .font_size(text_size(11.))
                        .color(colors::fg_secondary()),
                ),
        )
        .child(
            Button::new()
                .primary()
                .on_press(move |_| {
                    let svc = Arc::clone(&svc_send);
                    let name = name_send.clone();
                    let user_id = user_id_send.clone();
                    let mut status = status_send.clone();
                    spawn(async move {
                        // Elige un .mrpack (exportado desde Clusters) y envíalo.
                        let picked = rfd::AsyncFileDialog::new()
                            .add_filter(t("oneclient.friends.filter_instance"), &["mrpack", "json", "zip"])
                            .pick_file()
                            .await;
                        let Some(file) = picked else { return };
                        let path: PathBuf = file.path().to_path_buf();
                        status.write().push(tf("oneclient.friends.sending", &[&name]));
                        if let Err(e) = svc.send_file(&user_id, path).await {
                            status.write().push(tf("oneclient.friends.send_failed", &[&e.to_string()]));
                        }
                    });
                })
                .text(t("oneclient.friends.send")),
        )
        .child(
            Button::new()
                .danger()
                .on_press(move |_| {
                    let svc = Arc::clone(&svc_remove);
                    let user_id = user_id_remove.clone();
                    let mut status = status_remove.clone();
                    spawn(async move {
                        let _ = svc.remove_friend(&user_id).await;
                        status.write().push(t("oneclient.friends.removed").to_string());
                    });
                })
                .text(t("oneclient.friends.remove")),
        )
        .into_element()
}

/// Inicializa el servicio con la identidad de la cuenta de Minecraft activa.
async fn init_service() -> Result<Arc<FriendsService>, String> {
    let state = launcher::state().map_err(|e| e.to_string())?;
    let account = state
        .auth
        .default_account_for_launch()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no hay cuenta de Minecraft activa")?;
    let identity = oneclient_friends::identity::FriendIdentity::new(
        account.id.to_string(),
        account.username.clone(),
    );
    let inbox = oneclient_common::paths::friends_inbox_dir().unwrap_or_else(|_| PathBuf::from("/tmp/opencode/vinlauncher-inbox"));
    let store_path = oneclient_common::paths::friends_store_file()
        .unwrap_or_else(|_| PathBuf::from("/tmp/opencode/vinlauncher-friends.json"));
    FriendsService::new(store_path, inbox, identity)
        .await
        .map_err(|e| e.to_string())
}
