use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use super::model::{ServiceInfo, StatusResponse};

pub(super) fn fetch_status(info: &ServiceInfo) -> Result<StatusResponse, String> {
    let body = http_request(info, "GET", "/api/status", None)?;
    serde_json::from_str(&body).map_err(|error| format!("invalid status response: {error}"))
}

pub(super) fn disconnect_device(info: &ServiceInfo) -> Result<(), String> {
    let body = serde_json::json!({ "pin": info.pin }).to_string();
    http_request(info, "POST", "/api/disconnect", Some(&body)).map(|_| ())
}

fn http_request(
    info: &ServiceInfo,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], info.control_port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(350))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(350)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(350)))
        .map_err(|error| error.to_string())?;

    let request = build_http_request(info.control_port, method, path, body);
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    parse_http_response(&response)
}

fn build_http_request(control_port: u16, method: &str, path: &str, body: Option<&str>) -> String {
    let body = body.unwrap_or_default();
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{control_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn parse_http_response(response: &str) -> Result<String, String> {
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_string())?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| "invalid HTTP status".to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {}", body.trim()));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::{build_http_request, disconnect_device, fetch_status, parse_http_response};
    use crate::service::model::ServiceInfo;

    #[test]
    fn builds_request_with_body_length_and_close_header() {
        let request = build_http_request(4100, "POST", "/api/disconnect", Some(r#"{"pin":"123"}"#));

        assert!(request.starts_with("POST /api/disconnect HTTP/1.1\r\n"));
        assert!(request.contains("Host: 127.0.0.1:4100\r\n"));
        assert!(request.contains("Content-Length: 13\r\n"));
        assert!(request.contains("Connection: close\r\n"));
        assert!(request.ends_with(r#"{"pin":"123"}"#));
    }

    #[test]
    fn parses_successful_response_body() {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

        assert_eq!(parse_http_response(response).unwrap(), "ok");
    }

    #[test]
    fn rejects_error_status_with_body_context() {
        let response = "HTTP/1.1 409 Conflict\r\nContent-Length: 4\r\n\r\nbusy";

        assert_eq!(parse_http_response(response).unwrap_err(), "HTTP 409: busy");
    }

    #[test]
    fn rejects_malformed_response() {
        assert_eq!(
            parse_http_response("not http").unwrap_err(),
            "invalid HTTP response"
        );
        assert_eq!(
            parse_http_response("HTTP/1.1 nope\r\n\r\n").unwrap_err(),
            "invalid HTTP status"
        );
    }

    #[test]
    fn fetch_status_reads_json_from_local_service() {
        let body = r#"{"ok":true,"audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"stats":{"target":null,"device":null,"media_source":"tone","packets_sent":9,"bytes_sent":960,"capture_packets_dropped":0,"media_restarts":0,"media_started_ms":1,"last_packet_at_ms":2}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let (info, request_rx, server) = serve_once(response);

        let status = fetch_status(&info).unwrap();

        assert_eq!(status.stats.packets_sent, 9);
        assert!(request_rx.recv().unwrap().starts_with("GET /api/status "));
        server.join().unwrap();
    }

    #[test]
    fn disconnect_device_posts_pin_to_local_service() {
        let (info, request_rx, server) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\ndisconnected".to_string());

        disconnect_device(&info).unwrap();

        let request = request_rx.recv().unwrap();
        assert!(request.starts_with("POST /api/disconnect "));
        assert!(request.ends_with(r#"{"pin":"123456"}"#));
        server.join().unwrap();
    }

    fn serve_once(
        response: String,
    ) -> (ServiceInfo, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(500)))
                .unwrap();
            let mut buffer = [0_u8; 2048];
            let n = stream.read(&mut buffer).unwrap();
            request_tx
                .send(String::from_utf8_lossy(&buffer[..n]).to_string())
                .unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        let info = serde_json::from_str::<ServiceInfo>(&format!(
            r#"{{"event":"ready","control_url":"http://127.0.0.1:{port}","control_port":{port},"discovery_port":null,"pin":"123456","audio":{{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1}},"source":"tone","direct_target":null}}"#
        ))
        .unwrap();
        (info, request_rx, server)
    }
}
