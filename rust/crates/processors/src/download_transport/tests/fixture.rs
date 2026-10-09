//! A deterministic local HTTP(S) server for transport tests. Each connection
//! carries one request and is closed after the reply.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) target: String,
    /// Header names as sent, with their values, in order.
    pub(crate) headers: Vec<(String, String)>,
}
impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// Method, target, and headers with lowercased names, sorted.
    pub(crate) fn normalized(&self) -> String {
        let mut headers = self
            .headers
            .iter()
            .map(|(n, v)| format!("{}: {v}", n.to_ascii_lowercase()))
            .collect::<Vec<_>>();
        headers.sort();
        format!("{} {} {headers:?}", self.method, self.target)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Reply {
    status: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Sleep before each 16 KiB block of the body.
    pace: Option<Duration>,
    /// Close the connection after this many body bytes.
    cut: Option<usize>,
    /// Sleep before sending anything.
    delay: Option<Duration>,
}
impl Reply {
    pub(crate) fn new(status: &str) -> Self {
        Self {
            status: status.into(),
            ..Self::default()
        }
    }
    pub(crate) fn header(mut self, name: &str, value: impl ToString) -> Self {
        self.headers.push((name.into(), value.to_string()));
        self
    }
    /// Sets the body and a matching Content-Length.
    pub(crate) fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        let mut reply = self.header("Content-Length", body.len());
        reply.body = body;
        reply
    }
    pub(crate) fn raw_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
    pub(crate) fn pace(mut self, pause: Duration) -> Self {
        self.pace = Some(pause);
        self
    }
    pub(crate) fn cut(mut self, bytes: usize) -> Self {
        self.cut = Some(bytes);
        self
    }
    pub(crate) fn delay(mut self, pause: Duration) -> Self {
        self.delay = Some(pause);
        self
    }
}

type Handler = Arc<dyn Fn(&Request) -> Reply + Send + Sync>;

pub(crate) struct Server {
    /// `http://127.0.0.1:port` or the `https` equivalent.
    pub(crate) base: String,
    pub(crate) log: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    pub(crate) fn authority(&self) -> &str {
        self.base.split("://").nth(1).unwrap_or_default()
    }
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn serve(
    handler: impl Fn(&Request) -> Reply + Send + Sync + 'static,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let scheme = if tls.is_some() { "https" } else { "http" };
    let base = format!("{scheme}://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let handler: Handler = Arc::new(handler);
    let thread = {
        let (log, stop) = (log.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (handler, log, tls) = (handler.clone(), log.clone(), tls.clone());
                        connections.push(std::thread::spawn(move || {
                            let _ = connection(stream, tls, &handler, &log);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            for connection in connections {
                let _ = connection.join();
            }
        })
    };
    Server {
        base,
        log,
        stop,
        thread: Some(thread),
    }
}

fn connection(
    stream: TcpStream,
    tls: Option<Arc<rustls::ServerConfig>>,
    handler: &Handler,
    log: &Mutex<Vec<Request>>,
) -> std::io::Result<()> {
    // Accepted sockets inherit nonblocking mode on Windows.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    match tls {
        Some(config) => {
            let connection =
                rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
            let mut stream = rustls::StreamOwned::new(connection, stream);
            exchange(&mut stream, handler, log)?;
            stream.conn.send_close_notify();
            stream.flush()
        }
        None => {
            let mut stream = stream;
            exchange(&mut stream, handler, log)
        }
    }
}

fn exchange(
    stream: &mut impl ReadWrite,
    handler: &Handler,
    log: &Mutex<Vec<Request>>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let request = Request {
        method: first.next().unwrap_or_default().into(),
        target: first.next().unwrap_or_default().into(),
        headers: lines
            .filter_map(|line| line.split_once(':'))
            .map(|(n, v)| (n.to_string(), v.trim().to_string()))
            .collect(),
    };
    log.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    if let Some(pause) = reply.delay {
        std::thread::sleep(pause);
    }
    let mut out = format!("HTTP/1.1 {}\r\n", reply.status);
    for (name, value) in &reply.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    stream.write_all(out.as_bytes())?;
    if request.method != "HEAD" {
        let body = &reply.body[..reply.cut.unwrap_or(reply.body.len()).min(reply.body.len())];
        for block in body.chunks(16 << 10) {
            if let Some(pause) = reply.pace {
                std::thread::sleep(pause);
            }
            stream.write_all(block)?;
            stream.flush()?;
        }
    }
    stream.flush()
}

pub(crate) trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// A certificate authority and a server configuration for 127.0.0.1 signed
/// by it. Returns the server configuration and the CA certificate as PEM.
pub(crate) fn tls_identity() -> (Arc<rustls::ServerConfig>, String) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Russet transport test CA");
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let params =
        rcgen::CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".into()]).unwrap();
    let leaf = params.signed_by(&key, &issuer).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    (Arc::new(config), ca.pem())
}

/// Deterministic, non-repeating test content.
pub(crate) fn content(length: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_u32;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}
