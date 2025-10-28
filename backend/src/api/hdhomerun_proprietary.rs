use crate::api::model::AppState;
use crate::model::{AppConfig, HdHomeRunDeviceConfig};
use bytes::{Buf, BufMut, BytesMut};
use log::{error, info, trace};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio_util::sync::CancellationToken;

const HDHR_PROPRIETARY_PORT: u16 = 65001;

mod packet {
    pub const HDHOMERUN_TYPE_DISCOVER_REQ: u16 = 0x0002;
    pub const HDHOMERUN_TYPE_DISCOVER_RSP: u16 = 0x0003;
    pub const HDHOMERUN_TYPE_GETSET_REQ: u16 = 0x0004;
    pub const HDHOMERUN_TYPE_GETSET_RSP: u16 = 0x0005;

    pub const HDHOMERUN_TAG_DEVICE_TYPE: u8 = 0x01;
    pub const HDHOMERUN_TAG_DEVICE_ID: u8 = 0x02;
    pub const HDHOMERUN_TAG_TUNER_COUNT: u8 = 0x10;
    pub const HDHOMERUN_TAG_GETSET_NAME: u8 = 0x03;
    pub const HDHOMERUN_TAG_GETSET_VALUE: u8 = 0x04;
    pub const HDHOMERUN_TAG_ERROR_MESSAGE: u8 = 0x05;
    pub const HDHOMERUN_TAG_BASE_URL: u8 = 0x2A;
    pub const HDHOMERUN_TAG_LINEUP_URL: u8 = 0x27;

    pub const HDHOMERUN_DEVICE_TYPE_TUNER: u32 = 0x0000_0001;
    pub const HDHOMERUN_DEVICE_ID_WILDCARD: u32 = 0xFFFF_FFFF;
}

// --- UDP Discovery Logic ---

fn write_tlv_u32(buf: &mut BytesMut, tag: u8, value: u32) {
    buf.put_u8(tag);
    buf.put_u8(4);
    buf.put_u32(value);
}

fn write_tlv_u8(buf: &mut BytesMut, tag: u8, value: u8) {
    buf.put_u8(tag);
    buf.put_u8(1);
    buf.put_u8(value);
}

fn write_tlv_str(buf: &mut BytesMut, tag: u8, value: &str) {
    let bytes = value.as_bytes();
    buf.put_u8(tag);
    if bytes.len() < 0x80 {
        buf.put_u8(u8::try_from(bytes.len()).unwrap_or(0));
    } else {
        buf.put_u8(0x82);
        buf.put_u16(u16::try_from(bytes.len()).unwrap_or(0));
    }
    buf.put_slice(bytes);
}

fn build_discover_response(device: &HdHomeRunDeviceConfig, server_host: &str) -> Vec<u8> {
    let mut payload = BytesMut::new();
    let base_url = format!("http://{server_host}:{}", device.port);
    let lineup_url = format!("{base_url}/lineup.json");

    let device_id = u32::from_str_radix(&device.device_id, 16).unwrap_or(0);

    write_tlv_u32(
        &mut payload,
        packet::HDHOMERUN_TAG_DEVICE_TYPE,
        packet::HDHOMERUN_DEVICE_TYPE_TUNER,
    );
    write_tlv_u32(
        &mut payload,
        packet::HDHOMERUN_TAG_DEVICE_ID,
        device_id,
    );
    write_tlv_str(
        &mut payload,
        packet::HDHOMERUN_TAG_BASE_URL,
        &base_url,
    );
    write_tlv_u8(
        &mut payload,
        packet::HDHOMERUN_TAG_TUNER_COUNT,
        device.tuner_count,
    );
    write_tlv_str(
        &mut payload,
        packet::HDHOMERUN_TAG_LINEUP_URL,
        &lineup_url,
    );

    let mut response = BytesMut::new();
    response.put_u16(packet::HDHOMERUN_TYPE_DISCOVER_RSP);
    response.put_u16(u16::try_from(payload.len()).unwrap_or(0));
    response.put(payload);

    let crc = crc32fast::hash(&response);
    response.put_u32_le(crc);

    response.to_vec()
}

fn parse_tlv(cursor: &mut Cursor<&[u8]>) -> HashMap<u8, Vec<u8>> {
    let mut tags = HashMap::new();
    while cursor.position() < cursor.get_ref().len() as u64 {
        let mut tag_buf = [0u8; 1];
        if Read::read_exact(cursor, &mut tag_buf).is_err() {
            break;
        }

        let mut len_buf = [0u8; 1];
        if Read::read_exact(cursor, &mut len_buf).is_err() {
            break;
        }

        let len = if (len_buf[0] & 0x80) == 0 {
            len_buf[0] as usize
        } else {
            let ext_len_bytes = (len_buf[0] & 0x7F) as usize;
            let mut ext = vec![0u8; ext_len_bytes];
            if Read::read_exact(cursor, &mut ext).is_err() {
                break;
            }
            ext.iter().fold(0usize, |acc, b| (acc << 8) | (*b as usize))
        };


        if cursor.get_ref().len() <  usize::try_from(cursor.position()).unwrap_or(0) + len {
            break;
        }

        let mut val_buf = vec![0; len];
        if Read::read_exact(cursor, &mut val_buf).is_err() {
            break;
        }

        tags.insert(tag_buf[0], val_buf);
    }
    tags
}

async fn proprietary_discover_loop(
    socket: UdpSocket,
    app_config: Arc<AppConfig>,
    server_host: String,
) {
    let mut buf = [0; 1024];
    loop {
        let (len, remote_addr) = match socket.recv_from(&mut buf).await {
            Ok(result) => result,
            Err(e) => {
                error!("HDHomeRun proprietary socket error: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        let data = &buf[..len];
        if data.len() < 4 {
            continue;
        }

        let mut cursor = Cursor::new(data);
        let msg_type = cursor.get_u16();
        let _msg_len = cursor.get_u16();

        if msg_type == packet::HDHOMERUN_TYPE_DISCOVER_REQ {
            let tags = parse_tlv(&mut cursor);
            let requested_id = tags
                .get(&packet::HDHOMERUN_TAG_DEVICE_ID)
                .map(|id| u32::from_be_bytes(id.as_slice().try_into().unwrap_or([0u8; 4])));
            trace!("Received proprietary HDHomeRun discover from {remote_addr}");
            let hdhomerun_guard = app_config.hdhomerun.load();
            if let Some(hd_config) = &*hdhomerun_guard {
                if hd_config.enabled {
                    for device in &hd_config.devices {
                        if device.t_enabled {
                            let dev_id = u32::from_str_radix(&device.device_id, 16).unwrap_or(0);
                            let should_reply = match requested_id {
                                None => true,
                                Some(x) if x == packet::HDHOMERUN_DEVICE_ID_WILDCARD => true,
                                Some(x) => x == dev_id,
                            };
                            if should_reply {
                                let response = build_discover_response(device, &server_host);
                                if let Err(e) = socket.send_to(&response, remote_addr).await {
                                    error!("Failed to send proprietary discovery response to {remote_addr}: {e}");
                                } else {
                                    trace!("Sent proprietary discovery response for device '{}' to {remote_addr}", device.name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// --- TCP Get/Set Logic ---

async fn handle_tcp_connection(
    mut stream: tokio::net::TcpStream,
    _addr: SocketAddr,
    app_state: Arc<AppState>,
) {
    let mut buf = [0; 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => return, // Connection closed
            Ok(n) => {
                let request_data = &buf[..n];
                if request_data.len() < 4 {
                    continue;
                }

                let msg_type = u16::from_be_bytes([request_data[0], request_data[1]]);

                if msg_type == packet::HDHOMERUN_TYPE_GETSET_REQ {
                    let response = process_getset_request(request_data, &app_state).await;
                    if response.is_empty() {
                        error!("Protocol error or oinvalid request");
                        return;
                    }
                    if let Err(e) = stream.write_all(&response).await {
                        error!("Failed to write TCP response: {e}");
                        return;
                    }
                }
            }
            Err(e) => {
                error!("Failed to read from TCP socket: {e}");
                return;
            }
        }
    }
}

async fn process_getset_request(request: &[u8], app_state: &Arc<AppState>) -> Vec<u8> {
    let (data, crc_bytes) = request.split_at(request.len() - 4);
    let received_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    if crc32fast::hash(data) != received_crc {
        error!("Invalid CRC in HDHomeRun GET/SET request");
        return Vec::new();
    }

    let mut cursor = Cursor::new(&data[4..]); // Skip type and length
    let tags = parse_tlv(&mut cursor);

    let mut response_payload = BytesMut::new();

    if let Some(name_bytes) = tags.get(&packet::HDHOMERUN_TAG_GETSET_NAME) {
        let name = String::from_utf8_lossy(name_bytes);
        trace!("Received GET/SET for: {name}");

        let name_str = name.trim_end_matches('\0');
        write_tlv_str(
            &mut response_payload,
            packet::HDHOMERUN_TAG_GETSET_NAME,
            name_str,
        );

        match name_str {
            "/sys/model" => {
                write_tlv_str(
                    &mut response_payload,
                    packet::HDHOMERUN_TAG_GETSET_VALUE,
                    "hdhomerun4_atsc",
                );
            }
            s if s.starts_with("/tuner") && s.ends_with("/status") => {
                let rest = &s[6..];
                let end = rest.find('/').unwrap_or(rest.len());
                if let Ok(tuner_index) = rest[..end].parse::<usize>() {
                    let active_streams = app_state.active_users.active_streams().await;
                    let status_str = if let Some(stream_info) = active_streams.get(tuner_index) {
                        format!(
                            "ch={} lock=8vsb ss=98 snq=80 seq=90 bps=12345678 pps=1234",
                            stream_info.channel.title
                        )
                    } else {
                        "ch=none lock=none ss=0 snq=0 seq=0 bps=0 pps=0".to_string()
                    };
                    write_tlv_str(
                        &mut response_payload,
                        packet::HDHOMERUN_TAG_GETSET_VALUE,
                        &status_str,
                    );
                }
            }
            s if s.starts_with("/tuner") && s.ends_with("/vchannel") => {
                let rest = &s[6..];
                let end = rest.find('/').unwrap_or(rest.len());
                if let Ok(tuner_index) = rest[..end].parse::<usize>() {
                    let active_streams = app_state.active_users.active_streams().await;
                    let vchannel = if let Some(stream_info) = active_streams.get(tuner_index) {
                        stream_info.channel.title.clone()
                    } else {
                        "none".to_string()
                    };
                    write_tlv_str(
                        &mut response_payload,
                        packet::HDHOMERUN_TAG_GETSET_VALUE,
                        &vchannel,
                    );
                }
            }
            s if s.starts_with("/tuner") && s.ends_with("/lockkey") => {
                let err_msg = "ERROR: resource locked";
                write_tlv_str(
                    &mut response_payload,
                    packet::HDHOMERUN_TAG_ERROR_MESSAGE,
                    err_msg,
                );
            }
            _ => {
                write_tlv_str(
                    &mut response_payload,
                    packet::HDHOMERUN_TAG_GETSET_VALUE,
                    "",
                );
            }
        }
    }

    let mut response = BytesMut::new();
    response.put_u16(packet::HDHOMERUN_TYPE_GETSET_RSP);
    response.put_u16(u16::try_from(response_payload.len()).unwrap_or(0));
    response.put(response_payload);

    let crc = crc32fast::hash(&response);
    response.put_u32_le(crc);

    response.to_vec()
}

async fn proprietary_tcp_listener_loop(
    app_state: Arc<AppState>,
    cancel_token: CancellationToken,
) {
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, HDHR_PROPRIETARY_PORT));
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind proprietary HDHR TCP socket: {e}");
            return;
        }
    };
    info!("HDHomeRun proprietary TCP listener started on {addr}");

    loop {
        tokio::select! {
            biased;
            () = cancel_token.cancelled() => {
                info!("HDHomeRun proprietary TCP listener shutting down.");
                break;
            }
            Ok((socket, remote_addr)) = listener.accept() => {
                trace!("Accepted proprietary HDHR TCP connection from {remote_addr}");
                let app_state_clone = Arc::clone(&app_state);
                tokio::spawn(handle_tcp_connection(socket, remote_addr, app_state_clone));
            }
            else => {
                break;
            }
        }
    }
}

pub fn spawn_proprietary_tasks(
    app_state: Arc<AppState>,
    server_host: String,
    cancel_token: CancellationToken,
) {
    let app_config = Arc::clone(&app_state.app_config);
    let cancel_token_udp = cancel_token.clone();

    // UDP Discovery Task
    tokio::spawn(async move {
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, HDHR_PROPRIETARY_PORT));
        match UdpSocket::bind(addr).await {
            Ok(socket) => {
                if let Err(e) = socket.set_broadcast(true) {
                    error!("Failed to set broadcast on proprietary HDHR socket: {e}");
                    return;
                }
                info!("HDHomeRun proprietary discovery listener started on {addr}");

                tokio::select! {
                    () = proprietary_discover_loop(socket, app_config, server_host) => {},
                    () = cancel_token_udp.cancelled() => {
                        info!("HDHomeRun proprietary discovery listener shutting down.");
                    }
                }
            }
            Err(e) => {
                error!("Failed to bind proprietary HDHR UDP socket: {e}");
            }
        }
    });

    // TCP Get/Set Task
    tokio::spawn(proprietary_tcp_listener_loop(app_state, cancel_token));
}