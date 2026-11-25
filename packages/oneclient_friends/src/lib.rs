//! oneclient_friends — sistema de amigos serverless (P2P) para vinlauncher.
//!
//! Sin backend: dos launchers se conectan directamente entre sí aunque estén
//! en redes distintas. El "señalado" (quién es quién y dónde está) se hace
//! con códigos de invitación que se comparten por cualquier medio (Discord,
//! WhatsApp, etc.), y la transferencia de instancias va por la conexión P2P.

pub mod identity;
pub mod invite;
pub mod net;
pub mod service;
pub mod store;
pub mod transfer;
pub mod upnp;

pub use service::{FriendsEvent, FriendsService};
