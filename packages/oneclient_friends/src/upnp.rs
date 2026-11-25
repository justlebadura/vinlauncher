//! UPnP mínimo (SSDP + SOAP) para abrir un puerto TCP en el router.
//!
//! Implementado a mano para no añadir dependencias: basta con un socket UDP
//! para el discovery y reqwest para el SOAP. Si el router no soporta UPnP o
//! está desactivado, devuelve `None` sin romper nada.

use std::net::SocketAddrV4;
use std::time::Duration;

use tokio::net::UdpSocket;

const SSDP_ADDR: &str = "239.255.255.250:1900";
const SSDP_ST: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
/// Lease del mapeo UPnP en segundos (30 min; se renueva antes de expirar).
pub const LEASE_SECONDS: u32 = 1800;

/// Intenta mapear `port` (TCP) hacia esta máquina vía UPnP.
/// Devuelve la duración concedida si tuvo éxito.
pub async fn map_port(port: u16) -> Option<u32> {
    let control = discover_control_url().await?;
    match add_port_mapping(&control, port).await {
        Ok(lease) => {
            tracing::info!(port, lease, "UPnP: puerto mapeado");
            Some(lease)
        }
        Err(e) => {
            tracing::debug!(port, error = %e, "UPnP: no se pudo mapear el puerto");
            None
        }
    }
}

/// Elimina el mapeo (al cerrar limpiamente). Silencioso: es best-effort.
pub async fn unmap_port(port: u16) {
    if let Some(control) = discover_control_url().await {
        let _ = delete_port_mapping(&control, port).await;
    }
}

async fn discover_control_url() -> Option<String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).await.ok()?;
    let q = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {SSDP_ST}\r\n\r\n"
    );
    socket.send_to(q.as_bytes(), SSDP_ADDR).await.ok()?;
    let mut buf = vec![0u8; 4096];

    // Recoge respuestas durante ~2 s y prueba cada location encontrada.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut seen: Vec<String> = Vec::new();
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let n = match tokio::time::timeout(deadline - now, socket.recv(&mut buf)).await {
            Ok(Ok(n)) => n,
            _ => return None,
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        let location = text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("location:"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string());
        let Some(location) = location else { continue };
        if seen.iter().any(|s| s == &location) {
            continue;
        }
        seen.push(location.clone());
        if let Some(url) = control_url_from_description(&location).await {
            return Some(url);
        }
    }
}

async fn control_url_from_description(location: &str) -> Option<String> {
    let body = reqwest::get(location).await.ok()?.text().await.ok()?;
    // Busca el servicio WANIPConnection (1/2/3) y su controlURL.
    let service = body
        .split("<service>")
        .find(|chunk| chunk.contains("WANIPConnection"))?;
    let base = match location.split_once("://") {
        Some((scheme, rest)) => format!(
            "{}://{}",
            scheme,
            rest.split('/').next().unwrap_or(rest)
        ),
        None => location.to_string(),
    };
    let control = between(service, "<controlURL>", "</controlURL>")?.trim();
    if control.starts_with("http://") || control.starts_with("https://") {
        Some(control.to_string())
    } else if control.starts_with('/') {
        Some(format!("{base}{control}"))
    } else {
        Some(format!("{base}/{control}"))
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = text.split_once(start)?.1;
    Some(s.split_once(end)?.0)
}

fn soap_body(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body>{inner}</s:Body></s:Envelope>"
    )
}

async fn add_port_mapping(control: &str, port: u16) -> Result<u32, String> {
    let internal = local_ip_for_upnp()
        .map(|ip| ip.ip().to_string())
        .unwrap_or_else(|| "192.168.1.100".to_string());
    let inner = format!(
        "<u:AddPortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
         <NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{port}</NewExternalPort>\
         <NewProtocol>TCP</NewProtocol>\
         <NewInternalPort>{port}</NewInternalPort>\
         <NewInternalClient>{internal}</NewInternalClient>\
         <NewEnabled>1</NewEnabled>\
         <NewDescription>vinlauncher P2P</NewDescription>\
         <NewLeaseDuration>{LEASE_SECONDS}</NewLeaseDuration>\
         </u:AddPortMapping>"
    );
    let resp = reqwest::Client::new()
        .post(control)
        .header(
            "SOAPACTION",
            "\"urn:schemas-upnp-org:service:WANIPConnection:1#AddPortMapping\"",
        )
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .body(soap_body(&inner))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        Ok(LEASE_SECONDS)
    } else {
        let text = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {status}: {}", text.lines().next().unwrap_or("")))
    }
}

async fn delete_port_mapping(control: &str, port: u16) -> Result<(), String> {
    let inner = format!(
        "<u:DeletePortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
         <NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{port}</NewExternalPort>\
         <NewProtocol>TCP</NewProtocol>\
         </u:DeletePortMapping>"
    );
    let resp = reqwest::Client::new()
        .post(control)
        .header(
            "SOAPACTION",
            "\"urn:schemas-upnp-org:service:WANIPConnection:1#DeletePortMapping\"",
        )
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .body(soap_body(&inner))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

fn local_ip_for_upnp() -> Option<SocketAddrV4> {
    // Truco estándar: conectar un socket UDP a una IP externa revela la
    // interfaz de salida sin enviar paquetes.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(v4),
        std::net::SocketAddr::V6(_) => None,
    }
}
