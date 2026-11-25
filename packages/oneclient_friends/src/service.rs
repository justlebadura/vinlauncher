//! `FriendsService` — punto de entrada del sistema de amigos.
//!
//! Encapsula: listener P2P de fondo, códigos de invitación, lista de amigos
//! y transferencias de instancias. La UI solo habla con este servicio.
//!
//! NAT: además del dial directo, el servicio intenta abrir su puerto con
//! UPnP (mejor esfuerzo) y mantiene un bucle de keep-alive hacia cada amigo.
//! Como AMBOS lados se marcan mutuamente, si A no puede entrar al receptor B,
//! el keep-alive de B entra por la conexión que B abre hacia fuera, y A le
//! manda el archivo pendiente por ahí (sin servidor de señalización).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::identity::{FriendIdentity, FriendEntry};
use crate::invite::{self, InvitePayload};
use crate::net::{self, DEFAULT_PORT};
use crate::store::FriendsStore;
use crate::transfer::{self, Incoming};
use crate::upnp;

/// Eventos que el servicio emite hacia la UI.
#[derive(Debug, Clone)]
pub enum FriendsEvent {
    /// Llegó un archivo de un amigo; (nombre_amigo, ruta_destino, nombre_original).
    FileReceived(String, PathBuf, String),
    /// Transferencia enviada con éxito a (nombre_amigo, nombre_archivo).
    FileSent(String, String),
    /// Un amigo se conectó (nombre, user_id).
    PeerConnected(String, String),
    /// Envío directo falló; quedó en cola hasta que haya conexión (nombre, archivo).
    FileQueued(String, String),
    /// Error de fondo que la UI debería mostrar.
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub enum FriendsError {
    #[error("no hay cuenta de Minecraft activa")]
    NoAccount,
    #[error("código de invitación inválido")]
    InvalidInvite,
    #[error("el código no es una invitación para ti (destinatario distinto)")]
    WrongInvite,
    #[error("amigo no encontrado en la lista")]
    UnknownFriend,
    #[error("conexión fallida: {0}")]
    Connect(String),
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
}

pub struct FriendsService {
    pub store: Arc<FriendsStore>,
    /// Directorio donde aterrizan las instancias recibidas.
    pub inbox_dir: PathBuf,
    events_tx: mpsc::UnboundedSender<FriendsEvent>,
    events_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<FriendsEvent>>,
    my_identity: FriendIdentity,
    listen_port: u16,
    /// Transferencias pendientes por amigo (user_id, ruta) — se entregan en
    /// cuanto haya conexión en cualquier dirección.
    pending: tokio::sync::Mutex<Vec<(String, PathBuf)>>,
}

impl FriendsService {
    /// Crea el servicio y arranca el listener de fondo.
    ///
    /// * `store_path` — ruta del JSON de amigos (p. ej. bajo el directorio de datos).
    /// * `inbox_dir` — carpeta para instancias recibidas.
    /// * `my_identity` — identidad de la cuenta de Minecraft activa.
    pub async fn new(
        store_path: PathBuf,
        inbox_dir: PathBuf,
        my_identity: FriendIdentity,
    ) -> Result<Arc<Self>, FriendsError> {
        tokio::fs::create_dir_all(&inbox_dir).await?;
        let store = Arc::new(FriendsStore::load(store_path).await?);
        let (events_tx, events_rx) = mpsc::unbounded_channel();

        let (listen_port, listener) =
            net::bind_listener(DEFAULT_PORT).await.map_err(FriendsError::Io)?;

        let service = Arc::new(Self {
            store,
            inbox_dir,
            events_tx,
            events_rx: tokio::sync::Mutex::new(events_rx),
            my_identity,
            listen_port,
            pending: tokio::sync::Mutex::new(Vec::new()),
        });

        let svc = Arc::clone(&service);
        tokio::spawn(async move {
            svc.accept_loop(listener).await;
        });

        // NAT: intenta abrir el puerto con UPnP y renueva el lease.
        tokio::spawn(upnp_renew_loop(listen_port));

        // NAT: keep-alives mutuos (también entrega la cola de envíos).
        let svc = Arc::clone(&service);
        tokio::spawn(async move {
            svc.keep_alive_loop().await;
        });

        Ok(service)
    }

    pub fn listen_port(&self) -> u16 {
        self.listen_port
    }

    pub fn my_identity(&self) -> &FriendIdentity {
        &self.my_identity
    }

    /// Siguiente evento para la UI (None si el servicio terminó).
    pub async fn next_event(&self) -> Option<FriendsEvent> {
        self.events_rx.lock().await.recv().await
    }

    /// Genera un código de invitación para compartir con un amigo nuevo.
    /// Detecta la IP pública (servicio de eco externo, sin servidor propio).
    pub async fn create_invite(&self) -> Result<String, FriendsError> {
        let public_ip = invite::detect_public_ip().await;
        let payload = InvitePayload {
            v: 1,
            identity: self.my_identity.clone(),
            listen_port: self.listen_port,
            public_ip,
            token: invite::generate_token(),
        };
        Ok(invite::encode_invite(&payload))
    }

    /// Añade un amigo a partir de SU código de invitación y genera el código
    /// de respuesta (para que él también te tenga a ti).
    pub async fn add_from_invite(&self, code: &str) -> Result<FriendEntry, FriendsError> {
        let payload = invite::decode_invite(code).map_err(|_| FriendsError::InvalidInvite)?;
        if payload.identity.user_id == self.my_identity.user_id {
            return Err(FriendsError::WrongInvite); // es tu propio código
        }
        self.store.add(payload.identity.clone()).await?;
        // Endpoint para reconexión futura.
        if let Some(ip) = &payload.public_ip {
            self.store
                .touch(&payload.identity.user_id, Some(format!("{ip}:{}", payload.listen_port)))
                .await?;
        }
        let entry = FriendEntry {
            identity: payload.identity,
            label: None,
            last_endpoint: payload.public_ip.map(|ip| format!("{ip}:{}", payload.listen_port)),
            last_seen: None,
            added_at: 0,
        };
        Ok(entry)
    }

    /// Elimina un amigo por user_id.
    pub async fn remove_friend(&self, user_id: &str) -> Result<(), FriendsError> {
        self.store.remove(user_id).await?;
        Ok(())
    }

    /// Saca (si existe) una transferencia pendiente de la cola.
    async fn take_pending(&self, user_id: &str) -> Option<PathBuf> {
        let mut pending = self.pending.lock().await;
        if let Some(pos) = pending.iter().position(|(uid, _)| uid == user_id) {
            return Some(pending.remove(pos).1);
        }
        None
    }

    /// Envía un archivo (p. ej. un `.mrpack` exportado de un cluster) a un amigo.
    ///
    /// Primero intenta el dial directo al último endpoint conocido. Si falla
    /// (NAT estricto), deja el archivo en cola: se entregará en cuanto haya
    /// conexión en cualquier dirección (nuestro keep-alive o el de él).
    pub async fn send_file(self: &Arc<Self>, user_id: &str, path: PathBuf) -> Result<(), FriendsError> {
        let friend = self
            .store
            .list()
            .await
            .into_iter()
            .find(|f| f.identity.user_id == user_id)
            .ok_or(FriendsError::UnknownFriend)?;
        let endpoint = friend
            .last_endpoint
            .ok_or_else(|| FriendsError::Connect("sin dirección conocida".into()))?;

        let display_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("instancia.mrpack")
            .to_string();

        match net::connect_to_peer(&endpoint, &self.my_identity, "").await {
            Ok((mut _read, mut write, hs)) => {
                transfer::send_file(&mut write, &path, &display_name)
                    .await
                    .map_err(|e| FriendsError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
                self.events_tx
                    .send(FriendsEvent::FileSent(hs.identity.username, display_name))
                    .ok();
                self.store.touch(&hs.identity.user_id, Some(endpoint)).await?;
                Ok(())
            }
            Err(e) => {
                tracing::info!(friend = %friend.identity.username, error = %e, "dial directo falló; transferencia en cola");
                self.pending
                    .lock()
                    .await
                    .push((user_id.to_string(), path.clone()));
                self.events_tx
                    .send(FriendsEvent::FileQueued(
                        friend.identity.username.clone(),
                        display_name,
                    ))
                    .ok();
                // Intento inmediato de entrega (en vez de esperar el tick).
                let svc = Arc::clone(self);
                let user = user_id.to_string();
                tokio::spawn(async move {
                    svc.keep_alive_friend(&user).await;
                });
                Ok(())
            }
        }
    }

    /// Bucle de fondo: acepta conexiones entrantes.
    ///
    /// Regla del protocolo: quien MARCA (dialer) habla primero tras el
    /// handshake — o envía un archivo (header "file" + bytes) o un ping
    /// (header "ping"). El que acepta responde: si es ping y tiene algo en
    /// cola para ese amigo, le manda el archivo; si no, responde ping.
    async fn accept_loop(self: Arc<Self>, listener: TcpListener) {
        loop {
            let Ok((stream, _addr)) = listener.accept().await else {
                break;
            };
            let svc = Arc::clone(&self);
            tokio::spawn(async move {
                let peer = stream.peer_addr().ok();
                let (mut read, mut write, hs) = match net::accept_connection(stream, None).await {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::debug!(error = %e, "handshake entrante falló");
                        return;
                    }
                };
                let known = svc
                    .store
                    .list()
                    .await
                    .into_iter()
                    .find(|f| f.identity.user_id == hs.identity.user_id);
                let Some(friend) = known else {
                    tracing::info!(user = %hs.identity.username, "conexión de desconocido rechazada");
                    return;
                };
                // Actualiza el endpoint con la IP real desde la que vino.
                if let Some(p) = peer {
                    let port = friend
                        .last_endpoint
                        .as_deref()
                        .and_then(|ep| ep.rsplit(':').next())
                        .and_then(|port| port.parse::<u16>().ok())
                        .unwrap_or(p.port());
                    svc.store
                        .touch(&hs.identity.user_id, Some(format!("{}:{port}", p.ip())))
                        .await
                        .ok();
                }
                svc.events_tx
                    .send(FriendsEvent::PeerConnected(
                        hs.identity.username.clone(),
                        hs.identity.user_id.clone(),
                    ))
                    .ok();

                // El dialer habla primero: ¿archivo o ping?
                match transfer::read_header(&mut read).await {
                    Ok(h) if h.action == "ping" => {
                        // Nos marcó para saber si somos alcanzables; responder
                        // con un archivo pendiente si lo hay, si no, ping.
                        let queued = svc.take_pending(&hs.identity.user_id).await;
                        if let Some(path) = queued {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("instancia.mrpack")
                                .to_string();
                            match transfer::send_file(&mut write, &path, &name).await {
                                Ok(()) => {
                                    svc.events_tx
                                        .send(FriendsEvent::FileSent(
                                            hs.identity.username.clone(),
                                            name,
                                        ))
                                        .ok();
                                }
                                Err(e) => {
                                    svc.pending
                                        .lock()
                                        .await
                                        .push((hs.identity.user_id.clone(), path));
                                    svc.events_tx
                                        .send(FriendsEvent::Error(format!(
                                            "reenvío falló: {e}"
                                        )))
                                        .ok();
                                }
                            }
                        } else if let Err(e) = transfer::send_ping(&mut write).await {
                            tracing::debug!(error = %e, "no se pudo responder ping");
                        }
                    }
                    Ok(header) => {
                        match transfer::write_received(&mut read, &svc.inbox_dir, &header).await {
                            Ok(dest) => {
                                svc.events_tx
                                    .send(FriendsEvent::FileReceived(
                                        hs.identity.username.clone(),
                                        dest,
                                        header.name,
                                    ))
                                    .ok();
                                svc.store.touch(&hs.identity.user_id, None).await.ok();
                            }
                            Err(e) => {
                                svc.events_tx
                                    .send(FriendsEvent::Error(format!(
                                        "recepción de {} falló: {e}",
                                        hs.identity.username
                                    )))
                                    .ok();
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "cabecera entrante inválida");
                    }
                }
            });
        }
    }

    /// Bucle de keep-alive: cada pocos segundos intenta conectar con cada
    /// amigo. Sirve para (1) saber quién está alcanzable, (2) mantener NAT
    /// abierto, y (3) entregar transferencias en cola.
    async fn keep_alive_loop(self: Arc<Self>) {
        loop {
            let has_pending = !self.pending.lock().await.is_empty();
            let period = if has_pending { 5 } else { 20 };
            let friends = self.store.list().await;
            for friend in friends {
                if friend.last_endpoint.is_some() {
                    self.keep_alive_friend(&friend.identity.user_id).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(period)).await;
        }
    }

    /// Un ciclo de keep-alive hacia un amigo. Marcamos (enviamos ping) y
    /// esperamos su respuesta: ping (está alcanzable) o archivo pendiente.
    async fn keep_alive_friend(&self, user_id: &str) {
        let Some(friend) = self
            .store
            .list()
            .await
            .into_iter()
            .find(|f| f.identity.user_id == user_id)
        else {
            return;
        };
        let Some(endpoint) = friend.last_endpoint.clone() else {
            return;
        };
        let (mut read, mut write, _hs) =
            match net::connect_to_peer(&endpoint, &self.my_identity, "").await {
                Ok(x) => x,
                Err(e) => {
                    tracing::trace!(friend = %friend.identity.username, error = %e, "keep-alive falló");
                    return;
                }
            };
        if transfer::send_ping(&mut write).await.is_err() {
            return;
        }
        match transfer::receive_any(&mut read, &self.inbox_dir).await {
            Ok(Incoming::Ping) => {
                self.store.touch(user_id, Some(endpoint)).await.ok();
            }
            Ok(Incoming::File(dest, header)) => {
                self.store.touch(user_id, Some(endpoint)).await.ok();
                self.events_tx
                    .send(FriendsEvent::FileReceived(
                        friend.identity.username.clone(),
                        dest,
                        header.name,
                    ))
                    .ok();
            }
            Err(e) => {
                tracing::debug!(friend = %friend.identity.username, error = %e, "respuesta keep-alive inválida");
            }
        }
    }
}

/// Mapea el puerto con UPnP y renueva el lease cada mitad de duración.
async fn upnp_renew_loop(port: u16) {
    if upnp::map_port(port).await.is_none() {
        // Reintenta de vez en cuando por si el router cambia (o se activa).
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            if upnp::map_port(port).await.is_some() {
                break;
            }
        }
    }
    loop {
        tokio::time::sleep(Duration::from_secs(upnp::LEASE_SECONDS as u64 / 2)).await;
        if upnp::map_port(port).await.is_none() {
            tracing::debug!("UPnP: lease no renovado; se reintentará");
        }
    }
}
